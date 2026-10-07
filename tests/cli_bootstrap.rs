#![cfg(unix)]

mod support;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use support::command_data::assert_data_matches_its_command_form;
use support::fake_agent::{
    managed_agent_double, read_or_empty, write_fake_designer, write_fake_designer_for_user,
};
use support::{
    hold_workspace_lock, interruptible_stub, temp_workspace, terminate_and_wait, v8_runner_command,
    wait_for_file, write_shell_script as write_script, RunnerGuard,
};

const LOCAL_CONFIG_SCHEMA_MODEL_LINE: &str = "# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json";

fn write_designer_dump_script(path: &Path, calls_log: &Path, exit_code: i32) {
    let body = format!(
        r#"args="$*"
printf '%s\n' "$args" >> "{}"
out=""
target=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/Out" ]; then out="$arg"; fi
  if [ "$prev" = "/DumpConfigToFiles" ]; then target="$arg"; fi
  prev="$arg"
done
if [ -n "$out" ]; then printf 'designer log: %s\n' "$args" > "$out"; fi
# Чтение поколения ничего не выгружает: без каталога выгрузки файл лёг бы в корень.
if [ "{exit_code}" = "0" ] && [ -n "$target" ]; then
  mkdir -p "$target"
  printf '<Configuration />\n' > "$target/Configuration.xml"
fi
printf 'stderr: %s\n' "$args" >&2
exit {exit_code}"#,
        calls_log.display()
    );
    write_script(path, &body);
}

fn bootstrap_args<'a>(
    project_dir: &'a Path,
    platform_path: &'a Path,
    connection: &'a str,
) -> Vec<String> {
    vec![
        "bootstrap".to_owned(),
        "--project-dir".to_owned(),
        project_dir.display().to_string(),
        "--connection".to_owned(),
        connection.to_owned(),
        "--platform-version".to_owned(),
        "8.3.27".to_owned(),
        "--platform-path".to_owned(),
        platform_path.display().to_string(),
    ]
}

#[test]
fn bootstrap_empty_dir_creates_config_and_dumps_main_configuration() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File={tmp}/source ib"),
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(config.contains("format: DESIGNER"));
    assert!(!config.contains("builder:"));
    assert!(
        !config.contains("infobase"),
        "the project file names no base:\n{config}"
    );
    assert!(fs::read_to_string(project_dir.join("v8project.local.yaml"))
        .expect("local")
        .contains(&format!("connection: '/F \"{tmp}/source ib\"'")));
    assert!(config.contains("path: 'src/configuration'"));
    assert!(config.contains("version: '8.3.27'"));
    assert!(!config.contains("platform_path"));
    assert!(!config.contains("secret"));

    let local = fs::read_to_string(project_dir.join("v8project.local.yaml")).expect("local");
    assert!(local.starts_with(LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    assert!(local.contains("path: '"));
    assert!(local.contains(platform_path.display().to_string().as_str()));
    let gitignore = fs::read_to_string(project_dir.join(".gitignore")).expect("gitignore");
    assert_eq!(
        gitignore,
        "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
    );
    assert!(project_dir
        .join("src/configuration/Configuration.xml")
        .exists());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(read_or_empty(&agent.commands_log).contains("config dump-config-to-files"));
    assert!(calls.contains(&format!("/F {tmp}/source ib")));
    // Порт агента не объявлен: раннер берёт свободный на этот запуск, а не `1543`.
    let port = calls
        .split("/AgentPort ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("agent port");
    assert_ne!(port, "1543", "{calls}");
}

/// `--source-dir ./src`: `v8project.yaml` хранит написание пользователя, а argv платформы,
/// цель выгрузки в ответе и созданный каталог — путь без внутреннего `.`: та форма сломала
/// `ibcmd` (#4).
#[test]
fn clone_with_a_dotted_source_dir_hands_the_platform_a_clean_path() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());
    args.extend(["--source-dir".to_owned(), "./src".to_owned()]);

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(config.contains("path: './src'"), "{config}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let expected = fs::canonicalize(dir.path())
        .expect("canonical workspace")
        .join("project")
        .join("src")
        .display()
        .to_string();
    assert_eq!(payload["data"]["source_dir"], expected.as_str());
    assert_eq!(payload["data"]["dump_target_path"], expected.as_str());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(
        read_or_empty(&agent.commands_log).contains("config dump-config-to-files"),
        "{calls}"
    );
    assert!(!calls.contains("/./"), "{calls}");
    assert!(project_dir.join("src/Configuration.xml").exists());
}

#[test]
fn bootstrap_unquotes_simple_file_connection_path() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File=\"{tmp}/source ib\""),
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(!config.contains("infobase"), "{config}");
    assert!(fs::read_to_string(project_dir.join("v8project.local.yaml"))
        .expect("local")
        .contains(&format!("connection: '/F \"{tmp}/source ib\"'")));
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains(&format!("/F {tmp}/source ib")));
    assert!(!calls.contains(&format!("\\\"{tmp}/source ib\\\"")));
}

#[test]
fn bootstrap_json_success_keeps_credentials_in_local_overlay_only() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer_for_user(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
        "Admin",
    );
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.splice(
        0..0,
        [
            "--json-message".to_owned(),
            "--log-level".to_owned(),
            "debug".to_owned(),
        ],
    );
    args.extend([
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);
    let action_log = dir.path().join("actions.log");

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &action_log)
        .args(args)
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "clone");
    // Настоящая база для живой сверки не нужна: харнесс подкладывает утилиты платформы,
    // и команда доходит до собранного ответа.
    assert_data_matches_its_command_form(&payload, "`clone`");
    // Вторая половина обоих признаков. Без неё превью и боевой прогон неразличимы на
    // проводе: подмена `dumped` или `provider_dispatched` на `false` оставила бы весь
    // набор зелёным, а контракт различает «заведён» и «заведён и выгружен» именно ими.
    assert_eq!(payload["data"]["dumped"], true);
    assert_eq!(payload["data"]["provider_dispatched"], true);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("super-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("super-secret"));

    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(
        config.contains("# Generated by v8-runner clone\n"),
        "the header names the command that wrote the file:\n{config}"
    );
    assert!(!config.contains("Admin"));
    assert!(!config.contains("super-secret"));
    let local = fs::read_to_string(project_dir.join("v8project.local.yaml")).expect("local");
    assert!(local.contains("user: 'Admin'"));
    assert!(local.contains("password: 'super-secret'"));
    let log = fs::read_to_string(action_log).expect("action log");
    assert!(
        !log.contains("Admin"),
        "{}",
        log.lines()
            .filter(|line| line.contains("Admin"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!log.contains("super-secret"));
    // Учётные данные уходят агенту входом в сессию, а не ключами запуска: журнал называет
    // сессию и не называет ни пользователя, ни пароль.
    assert!(log.contains("opening agent session"), "{log}");
}

/// `INV.CLI.CLONE-FROM-WARNS-ON-A-BASE-OF-ANOTHER-COPY`: `clone --from` на файловой базе
/// другой рабочей копии не отказывает — проект заводится и выгружается, ответ предупреждает,
/// чья это база, а метка остаётся за прежней копией.
#[test]
fn clone_from_a_base_of_another_copy_runs_with_a_warning() {
    let bases = support::temp_workspace();
    let base = bases.path().join("source-ib");
    fs::create_dir_all(&base).expect("base");
    fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
    let marker = bases.path().join(".source-ib.v8-runner.owners.json");
    let foreign = serde_json::json!({
        "version": 2,
        "owners": [{
            "machine": "a".repeat(64),
            "host": "build-agent",
            "project": "/srv/elsewhere",
            "since": "2026-10-01T00:00:00Z"
        }]
    })
    .to_string();
    fs::write(&marker, &foreign).expect("marker");
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={}", base.display()),
    );
    args.insert(0, "--json-message".to_owned());

    let output = v8_runner_command().args(args).output().expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["dumped"], true, "{payload}");
    let warned = payload["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(Value::as_str)
        .any(|warning| {
            warning.contains("of another working copy") && warning.contains("/srv/elsewhere")
        });
    assert!(warned, "{payload}");
    assert_eq!(fs::read_to_string(&marker).expect("marker"), foreign);
}

#[test]
fn bootstrap_preserves_non_secret_connection_attributes() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File={tmp}/source-ib;Locale=ru"),
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(!config.contains("infobase"), "{config}");
    assert!(fs::read_to_string(project_dir.join("v8project.local.yaml"))
        .expect("local")
        .contains(&format!("connection: 'File={tmp}/source-ib;Locale=ru'")));
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains(&format!(
        "/IBConnectionString File={tmp}/source-ib;Locale=ru"
    )));
}

#[test]
fn bootstrap_rejects_existing_targets_without_force() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    for target in [
        "v8project.yaml",
        "v8project.local.yaml",
        "src/configuration",
    ] {
        let dir = temp_workspace();
        let project_dir = dir.path().join("project");
        let platform_path = dir.path().join("1cv8");
        let calls_log = dir.path().join("calls.log");
        write_designer_dump_script(&platform_path, &calls_log, 0);
        if target.ends_with("configuration") {
            fs::create_dir_all(project_dir.join(target)).expect("target dir");
        } else {
            fs::create_dir_all(&project_dir).expect("project dir");
            fs::write(project_dir.join(target), "existing").expect("target file");
        }

        let output = v8_runner_command()
            .args(bootstrap_args(
                &project_dir,
                &platform_path,
                &format!("File={tmp}/source-ib"),
            ))
            .output()
            .expect("run command");

        assert!(!output.status.success(), "target {target}");
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        assert!(!calls_log.exists());
    }
}

#[test]
fn bootstrap_does_not_write_local_overlay_when_gitignore_update_fails() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    fs::create_dir_all(project_dir.join(".gitignore")).expect("gitignore dir");
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    // Каталог с `.gitignore` не пуст: `--force` снимает этот отказ, чтобы дойти до записи.
    args.extend([
        "--force".to_owned(),
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("gitignore"));
    assert!(!project_dir.join("v8project.local.yaml").exists());
    assert!(!calls_log.exists());
}

#[test]
fn bootstrap_force_overwrites_existing_targets() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    fs::create_dir_all(project_dir.join("src/configuration")).expect("source dir");
    fs::write(project_dir.join("v8project.yaml"), "existing").expect("config");
    fs::write(project_dir.join("v8project.local.yaml"), "existing").expect("local");
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.push("--force".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(project_dir
        .join("src/configuration/Configuration.xml")
        .exists());
}

#[test]
fn bootstrap_rejects_embedded_connection_credentials() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    for connection in [
        &format!("File={tmp}/source-ib;Usr=Admin;Pwd=secret"),
        &format!("/F {tmp}/source-ib /N Admin /P secret"),
        "/S server/ref /N=Admin /P=secret",
    ] {
        let dir = temp_workspace();
        let project_dir = dir.path().join("project");
        let platform_path = dir.path().join("1cv8");
        let output = v8_runner_command()
            .args(bootstrap_args(&project_dir, &platform_path, connection))
            .output()
            .expect("run command");

        assert!(!output.status.success(), "connection {connection}");
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("embedded credentials"));
        assert!(!project_dir.join("v8project.yaml").exists());
    }
}

#[test]
fn bootstrap_rejects_global_config_flag_in_text_mode() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");

    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "bootstrap",
            "--project-dir",
            &project_dir.display().to_string(),
            "--connection",
            &format!("File={tmp}/source-ib"),
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &platform_path.display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported for `clone`"));
    assert!(!project_dir.join("v8project.yaml").exists());
}

#[test]
fn bootstrap_rejects_global_config_flag_in_json_mode() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");

    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "--json-message",
            "bootstrap",
            "--project-dir",
            &project_dir.display().to_string(),
            "--connection",
            &format!("File={tmp}/source-ib"),
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &platform_path.display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "clone");
    assert_eq!(payload["error"]["kind"], "validation");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("not supported for `clone`"));
    assert!(!project_dir.join("v8project.yaml").exists());
}

#[test]
fn bootstrap_failed_dump_redacts_secrets_in_outputs() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer_for_user(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
        "Admin",
    );
    agent
        .fail_dump
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let action_log = dir.path().join("actions.log");
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.splice(
        0..0,
        [
            "--json-message".to_owned(),
            "--log-level".to_owned(),
            "debug".to_owned(),
        ],
    );
    args.extend([
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &action_log)
        .args(args)
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("Admin"));
    assert!(!stdout.contains("super-secret"));
    assert!(!stderr.contains("Admin"));
    assert!(!stderr.contains("super-secret"));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "clone");
    assert_eq!(payload["data"]["dumped"], false);
    // Упавшая выгрузка — не превью: платформа запускалась и отказала. Без этой строки
    // подмена признака на `false` сделала бы отказ неотличимым от плана.
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    assert!(payload["data"]["path"]
        .as_str()
        .expect("path")
        .contains("v8project.yaml"));
    assert!(payload["data"]["dump_target_path"]
        .as_str()
        .expect("target")
        .contains("src/configuration"));
    let log = fs::read_to_string(action_log).expect("action log");
    assert!(
        !log.contains("Admin"),
        "{}",
        log.lines()
            .filter(|line| line.contains("Admin"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!log.contains("super-secret"));
}

/// `clone` берёт замок `workPath` нового проекта раньше первой записи: занятый каталог —
/// отказ `workspace_busy` одним сообщением, проекта нет и платформа не запускалась.
#[test]
fn clone_refuses_a_busy_workspace_before_writing_the_project() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    hold_workspace_lock(&project_dir.join("build"));

    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());
    let output = v8_runner_command()
        .args(&args)
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(3));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("one json document");
    assert_eq!(payload["command"], "clone");
    assert_eq!(payload["error"]["code"], "workspace_busy");
    assert_eq!(payload["error"]["kind"], "workspace");
    assert_eq!(payload["steps"][0]["name"], "workspace lock");
    assert!(payload["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("cannot start clone")));
    assert!(!project_dir.join("v8project.yaml").exists());
    assert!(!project_dir.join("v8project.local.yaml").exists());
    assert!(!calls_log.exists(), "the platform must not be started");

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File={tmp}/source-ib"),
        ))
        .output()
        .expect("run command");
    assert_eq!(output.status.code(), Some(3));
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        printed.matches("cannot start clone").count(),
        1,
        "{printed}"
    );
}

/// SIGTERM посреди выгрузки — отмена, как у остальных команд: раннер снимает выгрузку,
/// отпускает замок и отвечает. Без перехвата сигнал убил бы раннер, и файл
/// владельца замка остался бы в `build` — следующая команда проекта отказывала бы до
/// ручной чистки.
#[test]
fn an_interrupted_clone_leaves_no_workspace_lock_behind() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let started = dir.path().join("dump-started");
    let release = dir.path().join("dump-release");
    let stderr = dir.path().join("stderr.log");
    write_script(&platform_path, &interruptible_stub(&started, &release));

    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());
    let mut runner = RunnerGuard(
        v8_runner_command()
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(fs::File::create(&stderr).expect("stderr log"))
            .spawn()
            .expect("spawn clone"),
    );
    let timeout = std::time::Duration::from_secs(30);
    assert!(wait_for_file(&started, timeout), "the dump never started");
    let stopped = terminate_and_wait(&mut runner.0, timeout);
    fs::write(&release, "").expect("release a stray dump");
    assert!(
        stopped,
        "clone did not stop after SIGTERM: {}",
        fs::read_to_string(&stderr).unwrap_or_default()
    );

    assert!(
        !project_dir
            .join("build")
            .join(".v8-runner.workspace.lock")
            .exists(),
        "the lock owner file must not outlive the command"
    );
    let status = runner.0.wait().expect("clone status");
    let mut stdout = String::new();
    std::io::Read::read_to_string(runner.0.stdout.as_mut().expect("piped stdout"), &mut stdout)
        .expect("stdout");
    // Ответ есть — значит, сигнал раннер не убил, а отменил: выгрузка снята и названа.
    let payload: Value = serde_json::from_str(&stdout).expect("one json document");
    assert_eq!(payload["ok"], false, "{payload}");
    assert_eq!(payload["error"]["code"], "cancelled", "{payload}");
    assert_eq!(status.code(), Some(4), "{payload}");
}

/// Память о базе минимального проекта, как после её создания раннером.
fn remember_minimal(dir: &Path, work: &Path) {
    support::memory::remember_base(
        work,
        "origin",
        support::memory::Base::File(&dir.join("ib")),
        &[support::memory::Set::configuration(
            "main",
            &dir.join("project"),
        )],
    );
}

fn write_minimal_config(dir: &Path) -> PathBuf {
    let config_path = dir.join("v8project.yaml");
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");
    config_path
}

#[test]
fn missing_config_in_text_mode_returns_validation_error_on_stderr() {
    let output = v8_runner_command()
        .args(["--config", "/definitely/missing/v8project.yaml", "build"])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("config file not found"));
}

#[test]
fn missing_config_in_json_mode_keeps_error_envelope_shape() {
    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["duration_ms"], 0);
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(
        payload["data"]["message"],
        "config file not found: /definitely/missing/v8project.yaml"
    );
}

#[test]
fn default_config_path_uses_v8project_yaml_from_current_dir() {
    let dir = temp_workspace();
    let _config_path = write_minimal_config(dir.path());
    remember_minimal(dir.path(), &dir.path().join("work"));

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "build"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "push");
}

#[test]
fn default_config_path_applies_sibling_local_overlay() {
    let dir = temp_workspace();
    let _config_path = write_minimal_config(dir.path());
    let local_work_path = dir.path().join("local-work");
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "workPath: local-work\n",
    )
    .expect("local overlay");
    remember_minimal(dir.path(), &local_work_path);

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "build"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "push");
    assert!(local_work_path.exists());
}

#[test]
fn unsupported_main_config_shape_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let mut config = fs::read_to_string(&config_path).expect("config");
    config.push_str("tools:\n  platform:\n    typo: value\n");
    fs::write(&config_path, config).expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("config contains unsupported key or value"));
}

#[test]
fn unsupported_local_overlay_shape_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "tools:\n  platform:\n    typo: value\n",
    )
    .expect("local overlay");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("local config overlay contains unsupported key or value"));
}

#[test]
fn action_logging_failure_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let log_path = dir.path().join("action-log-as-dir");
    fs::create_dir_all(&log_path).expect("log dir");

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &log_path)
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "runtime_failure");
    assert_eq!(payload["error"]["kind"], "runtime");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("failed to open action log file"));
}

#[test]
fn test_module_pre_dispatch_validation_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "test",
            "yaxunit",
            "module",
            "   ",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "test");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(
        payload["data"]["message"],
        "test module requires a non-empty module name"
    );
}

#[test]
fn artifacts_pre_dispatch_validation_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "make",
            "--output",
            &dir.path().join("out.cf").display().to_string(),
            "--source-set",
            "missing",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "make");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(payload["data"]["message"], "unknown source-set 'missing'");
}

#[test]
fn mcp_rejects_clean_before_execution_flag() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--clean-before-execution",
            "mcp",
            "serve",
            "stdio",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--clean-before-execution is not supported for MCP transports"));
}

#[test]
fn legacy_top_level_connection_is_rejected_in_json_mode() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nconnection: 'File={tmp}/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("legacy top-level key 'connection'"));
}

#[test]
fn legacy_top_level_credentials_is_rejected_in_json_mode() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File={tmp}/ib'\ncredentials:\n  user: Admin\n  password: secret\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("legacy top-level key 'credentials'"));
}

#[test]
fn top_level_execution_timeout_seconds_is_rejected_in_json_mode() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nexecution_timeout_seconds: 300\nformat: DESIGNER\ninfobase:\n  connection: 'File={tmp}/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("top-level key 'execution_timeout_seconds'"));
    assert!(message.contains("tests.execution_timeout_seconds"));
}

/// Превью называет проект, которого ещё нет, и не заводит его.
///
/// Четыре пути в ответе — те, что были бы написаны; на диске после вызова нет ни одного,
/// как и самого каталога проекта. Признак `provider_dispatched: false` говорит о том же
/// вторым полем: вызывающему не приходится выводить отсутствие запуска из отсутствия
/// значения.
#[test]
fn clone_preview_names_the_project_it_would_write_and_writes_nothing() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());
    args.push("--dry-run".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(0));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "clone");
    assert_data_matches_its_command_form(&payload, "`clone --dry-run`");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert_eq!(payload["data"]["dumped"], false);
    assert_eq!(payload["data"]["ok"], true);
    // Утилиту, которой выгружал бы, превью называет словами: квитанции эта форма не несёт.
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(
        message.contains(&platform_path.display().to_string()),
        "превью не назвало утилиту: {message}"
    );

    // Четыре пути — обещание записи `DEC.2026-09-23.CLONE-SHOWS-THE-PROJECT-IT-WOULD-WRITE`
    // и контракта: их называют поимённо и проверяют, что ни одного нет. Пятый путь
    // ответа — `dump_target_path` — цель выгрузки, а не написанный файл.
    let named = [
        &payload["data"]["path"],
        &payload["data"]["local_path"],
        &payload["data"]["gitignore_path"],
        &payload["data"]["source_dir"],
    ];
    assert_eq!(named.len(), 4);
    for path in named {
        let path = std::path::Path::new(path.as_str().expect("path is a string"));
        assert!(!path.exists(), "превью написало {}", path.display());
    }

    assert!(
        !project_dir.exists(),
        "превью завело каталог проекта: {:?}",
        fs::read_dir(&project_dir)
            .map(|entries| entries
                .flatten()
                .map(|entry| entry.path())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );
    assert!(!calls_log.exists(), "превью запустило платформу");
}

/// Отсутствие платформы отказывает и у `clone`, и отказ называет искомое.
///
/// Поиск утилиты у превью и у боевого прогона один и тот же — его делает `dump_config`, и
/// превью отличается от применения только признаком в запросе. Здесь проверяется превью;
/// равенство самих отказов держит не этот тест, а общий шов.
#[test]
fn clone_preview_refuses_without_a_platform_and_names_what_it_looked_for() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    // Каталог есть, утилиты в нём нет: подсказка пути замыкает поиск, и в PATH он не уходит.
    let platform_dir = dir.path().join("platform");
    fs::create_dir_all(platform_dir.join("bin")).expect("platform dir");
    let mut args = bootstrap_args(
        &project_dir,
        &platform_dir,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());
    args.push("--dry-run".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["provider_dispatched"], false);
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("1cv8"),
        "отказ не назвал искомое: {message}"
    );
    assert!(!project_dir.exists(), "отказ превью завёл каталог проекта");
}

/// Текстовый вывод превью не называет проект заведённым: пути он печатает те же, и без
/// ярлыка их нельзя отличить от написанных. Признак берётся у запроса, поэтому проверка
/// падает и тогда, когда ярлык начинают выводить из чего-то другого.
#[test]
fn clone_preview_text_output_does_not_announce_a_cloned_project() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.push("--dry-run".to_owned());

    let preview = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert_eq!(preview.status.code(), Some(0));
    let text = String::from_utf8_lossy(&preview.stdout);
    assert!(
        text.contains("Project clone planned, nothing written"),
        "превью не назвало себя планом: {text}"
    );
    assert!(
        !text.contains("Project cloned successfully"),
        "превью назвало проект заведённым: {text}"
    );

    // Боевой прогон по-прежнему говорит о заведённом проекте: иначе проверка держала бы
    // не ярлык превью, а его исчезновение у обоих.
    let real = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File={tmp}/source-ib"),
        ))
        .output()
        .expect("run command");
    let text = String::from_utf8_lossy(&real.stdout);
    assert!(
        text.contains("Project cloned successfully"),
        "боевой прогон потерял свой ярлык: {text}"
    );
}

/// Путь в ответе разрешается так же, как прежде разрешал `canonicalize` после создания:
/// символьная ссылка раскрывается. Проверка стоит здесь потому, что создание каталога из
/// разрешения пути ушло, и подмена разрешателя иначе осталась бы незамеченной.
#[test]
fn clone_resolves_a_symlinked_project_directory_to_its_target() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    fs::create_dir_all(&target).expect("target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    let mut args = bootstrap_args(&link, &platform_path, &format!("File={tmp}/source-ib"));
    args.insert(0, "--json-message".to_owned());
    args.push("--dry-run".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(0));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let canonical_target = fs::canonicalize(&target).expect("canonical target");
    assert_eq!(
        payload["data"]["path"],
        Value::from(
            canonical_target
                .join("v8project.yaml")
                .display()
                .to_string()
        )
    );
}

/// Проект — подкаталог чужого репозитория (монорепо, `git init` в домашнем каталоге):
/// `clone` пишет шаблоны в `.gitignore` каталога проекта, а корневой `.gitignore`
/// репозитория не трогает. Форма называет именно файл проекта.
#[test]
fn clone_into_a_subdirectory_of_a_repository_writes_the_project_gitignore() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["init", "-q", "-b", "main", "."])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git init failed");
    let root_gitignore = repo.join(".gitignore");
    fs::write(&root_gitignore, "target/\n").expect("root gitignore");
    let project_dir = repo.join("apps").join("erp");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.insert(0, "--json-message".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let project_gitignore = fs::canonicalize(&project_dir)
        .expect("canonical project dir")
        .join(".gitignore");
    assert_eq!(
        payload["data"]["gitignore_path"],
        Value::from(project_gitignore.display().to_string())
    );
    assert_eq!(
        fs::read_to_string(&root_gitignore).expect("root gitignore"),
        "target/\n"
    );
    assert_eq!(
        fs::read_to_string(&project_gitignore).expect("project gitignore"),
        "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
    );
}

/// Источник клона называет `--from`, как в словаре сайта: позиционного адреса у `clone`
/// нет, а прежний ключ `--connection` в справке не печатается.
#[test]
fn clone_takes_its_source_from_the_from_key() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );

    let output = v8_runner_command()
        .args([
            "clone",
            "--project-dir",
            &project_dir.display().to_string(),
            "--from",
            &format!("File={tmp}/source-ib"),
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &platform_path.display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let local = fs::read_to_string(project_dir.join("v8project.local.yaml")).expect("local");
    assert!(
        local.contains(&format!("connection: '/F \"{tmp}/source-ib\"'")),
        "{local}"
    );
    assert!(fs::read_to_string(calls_log)
        .expect("calls")
        .contains(&format!("/F {tmp}/source-ib")));

    let help = v8_runner_command()
        .args(["clone", "--help"])
        .output()
        .expect("run help");
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--from <CONNECTION>"), "{help}");
    assert!(!help.contains("--connection"), "{help}");
}

/// Запускает `clone` в формате конверта и разбирает его ответ.
fn run_clone_json(args: Vec<String>) -> (Option<i32>, Value) {
    let mut args = args;
    args.insert(0, "--json-message".to_owned());
    let output = v8_runner_command()
        .args(&args)
        .output()
        .expect("run command");
    let payload = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "one json document ({error}):\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code(), payload)
}

/// `clone` пишет проект только в пустой каталог. Непустой — отказ `invalid_argument` до
/// записи: ни проекта, ни `.gitignore`, ни каталога исходников, платформа не запускалась.
/// Превью отказывает так же. Занятый замок отвечает раньше: сначала `workspace_busy`.
#[test]
fn clone_refuses_a_non_empty_directory_before_writing_anything() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::write(project_dir.join("notes.txt"), "user file").expect("user file");

    for preview in [false, true] {
        let mut args = bootstrap_args(
            &project_dir,
            &platform_path,
            &format!("File={tmp}/source-ib"),
        );
        if preview {
            args.insert(0, "--dry-run".to_owned());
        }
        let (code, payload) = run_clone_json(args);

        assert_eq!(code, Some(2), "preview {preview}: {payload}");
        assert_eq!(payload["command"], "clone");
        assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
        assert_eq!(payload["error"]["kind"], "validation", "{payload}");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(message.contains("clone target is not empty"), "{message}");
        assert!(message.contains("notes.txt"), "{message}");
        assert!(message.contains("--force"), "{message}");
    }

    assert!(!project_dir.join("v8project.yaml").exists());
    assert!(!project_dir.join("v8project.local.yaml").exists());
    assert!(!project_dir.join(".gitignore").exists());
    assert!(!project_dir.join("src").exists());
    assert!(!calls_log.exists(), "the platform must not be started");
    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).expect("user file"),
        "user file"
    );

    hold_workspace_lock(&project_dir.join("build"));
    let (code, payload) = run_clone_json(bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    ));
    assert_eq!(code, Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "workspace_busy", "{payload}");
}

/// `workPath` нового проекта не в счёт, только пока в нём лишь замок и журналы этого
/// запуска: старый файл в `build` делает каталог непустым — отказ `invalid_argument`.
#[test]
fn clone_refuses_a_work_path_holding_a_foreign_file() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    fs::create_dir_all(project_dir.join("build")).expect("work dir");
    fs::write(project_dir.join("build/stale.txt"), "old dump").expect("stale file");

    let (code, payload) = run_clone_json(bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    ));

    assert_eq!(code, Some(2), "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("clone target is not empty"), "{message}");
    assert!(message.contains("build"), "{message}");
    assert!(!project_dir.join("v8project.yaml").exists());
    assert!(!project_dir.join(".gitignore").exists());
    assert!(!calls_log.exists(), "the platform must not be started");
}

/// Каталог, где нет ничего, кроме `.git`, пуст: `clone` в свежий репозиторий проходит.
#[test]
fn clone_into_a_directory_holding_only_git_writes_the_project() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    fs::create_dir_all(project_dir.join(".git")).expect("git dir");
    fs::write(project_dir.join(".git/HEAD"), "ref: refs/heads/master\n").expect("head");

    let (code, payload) = run_clone_json(bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    ));

    assert_eq!(code, Some(0), "{payload}");
    assert_eq!(payload["data"]["dumped"], true, "{payload}");
    assert!(project_dir.join("v8project.yaml").exists());
    assert!(project_dir
        .join("src/configuration/Configuration.xml")
        .exists());
}

/// `--force` снимает отказ по непустому каталогу: чужой файл остаётся, проект пишется.
#[test]
fn clone_force_writes_the_project_into_a_non_empty_directory() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    let agent = managed_agent_double();
    write_fake_designer(
        &platform_path,
        &calls_log,
        &agent.pid_file,
        &agent.base_dir_file,
    );
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::write(project_dir.join("notes.txt"), "user file").expect("user file");
    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.push("--force".to_owned());

    let (code, payload) = run_clone_json(args);

    assert_eq!(code, Some(0), "{payload}");
    assert_eq!(payload["data"]["dumped"], true, "{payload}");
    assert!(project_dir.join("v8project.yaml").exists());
    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).expect("user file"),
        "user file"
    );
}

/// Клон в репозиторий, где в каталоге исходников лежит работа вне учёта: сторож отказывает,
/// и выход у `clone` один — сохранить работу. Ключа согласия на уничтожение у него нет:
/// `clone --force` касается непустого каталога, и совет повторить с ним зациклил бы агента.
#[test]
fn a_clone_refusal_does_not_offer_force() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    let source_dir = project_dir.join("src").join("configuration");
    fs::create_dir_all(&source_dir).expect("source dir");
    let vcs = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&project_dir)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    };
    vcs(&["init", "-q", "-b", "main", "."]);
    vcs(&["config", "user.email", "test@example.com"]);
    vcs(&["config", "user.name", "Test"]);
    fs::write(project_dir.join("README.md"), "readme\n").expect("readme");
    vcs(&["add", "-A"]);
    vcs(&["commit", "-qm", "readme"]);
    fs::write(source_dir.join("hand-written.xml"), "mine\n").expect("hand-written");

    let mut args = bootstrap_args(
        &project_dir,
        &platform_path,
        &format!("File={tmp}/source-ib"),
    );
    args.push("--force".to_owned());
    let (code, payload) = run_clone_json(args);

    assert_eq!(code, Some(2), "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("refusing to replace"), "{message}");
    assert!(message.contains("hand-written.xml"), "{message}");
    assert!(
        message.contains("commit or stash them and run the same command again"),
        "{message}"
    );
    assert!(!message.contains("--force"), "{message}");
    assert!(source_dir.join("hand-written.xml").is_file());
}
