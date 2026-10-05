#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::fake_agent::{start_fake_agent, FakeAgent, AGENT_PASSWORD};
use support::{
    interruptible_stub, temp_workspace, terminate_and_wait, v8_runner_command, wait_for_file,
    write_shell_script, write_shell_script_atomically, RunnerGuard,
};

struct Project {
    _dir: tempfile::TempDir,
    config: PathBuf,
    sources: PathBuf,
    work: PathBuf,
    calls: PathBuf,
    binary: PathBuf,
    extension: bool,
}

/// Поддельная платформа, которая выгружает в каталог из командной строки два файла.
fn dumping_platform(calls: &Path) -> String {
    format!(
        r#"printf '%s\n' "$*" >> '{}'
target=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/DumpConfigToFiles' ]; then target="$arg"; fi
  previous="$arg"
done
if [ "$1" = 'config' ] && [ "$2" = 'export' ]; then target="$previous"; fi
if [ -n "$target" ]; then
  mkdir -p "$target"
  printf '<Configuration/>\n' > "$target/Configuration.xml"
  printf 'Procedure Published()\nEndProcedure\n' > "$target/Module.bsl"
fi
exit 0"#,
        calls.display()
    )
}

fn project(provider: &str, extension: bool) -> Project {
    let dir = temp_workspace();
    let root = dir.path();
    let config = root.join("v8project.yaml");
    let sources = root.join("sources");
    let work = root.join("work");
    let calls = root.join("calls.log");
    fs::create_dir_all(&sources).expect("sources");
    fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
    fs::write(sources.join("old.txt"), "local contents before pull").expect("old source");
    let base_source = if extension {
        fs::create_dir_all(root.join("base")).expect("base source");
        fs::write(root.join("base/Configuration.xml"), "<Configuration/>\n").expect("base marker");
        "  - name: base\n    type: CONFIGURATION\n    path: base\n"
    } else {
        ""
    };
    let binary = root.join(if provider == "ibcmd" { "ibcmd" } else { "1cv8" });
    write_shell_script(&binary, &dumping_platform(&calls));
    let agent_yaml = if provider == "agent" {
        let base = root.join("agent-base");
        fs::create_dir_all(base.join("0")).expect("agent user dir");
        fs::write(
            base.join("agentbasedir.json"),
            r#"{"usersInfo":[{"name":"","dir":"0"}]}"#,
        )
        .expect("agent map");
        let port = start_fake_agent(FakeAgent::new(
            true,
            calls.clone(),
            Some(base.clone()),
            root.join("base-dir.txt"),
            root.join("designer.pid"),
        ));
        format!(
            "  designer_agent:\n    attach: 127.0.0.1:{port}\n    base-dir: '{}'\n",
            base.display()
        )
    } else {
        String::new()
    };
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nproviders:\n  dump: {provider}\n  build: {provider}\nsource-set:\n{base_source}  - name: main\n    type: {}\n    path: sources\ntools:\n  platform:\n    path: '{}'\n{agent_yaml}",
            work.display(),
            if extension { "EXTENSION" } else { "CONFIGURATION" },
            binary.display(),
        ),
    ).expect("config");
    fs::write(root.join("v8project.local.yaml"), format!(
        "infobases:\n  origin:\n    connection: 'File={}'\n    password: '{AGENT_PASSWORD}'\n  second:\n    connection: 'File={}'\n    password: '{AGENT_PASSWORD}'\n",
        root.join("ib").display(), root.join("second-ib").display(),
    )).expect("local config");
    Project {
        _dir: dir,
        config,
        sources,
        work,
        calls,
        binary,
        extension,
    }
}

fn run(project: &Project, args: &[&str]) -> Output {
    v8_runner_command()
        .arg("--config")
        .arg(&project.config)
        .arg("--json-message")
        .args(args)
        .output()
        .expect("run CLI")
}

fn succeeded(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn pull(project: &Project) {
    if project.extension {
        succeeded(run(project, &["pull", "--force", "--source-set", "base"]));
    }
    let selector = if project.extension {
        "--extension"
    } else {
        "--source-set"
    };
    succeeded(run(project, &["pull", "--force", selector, "main"]));
}

fn assert_push_skips(project: &Project) {
    let before = fs::read_to_string(&project.calls).unwrap_or_default();
    let response = succeeded(run(project, &["push"]));
    let steps = response["data"]["steps"].as_array().expect("push steps");
    assert!(!steps.is_empty(), "{response}");
    assert!(
        steps.iter().all(|step| step["mode"] == "skipped"),
        "{response}"
    );
    assert_eq!(
        fs::read_to_string(&project.calls).unwrap_or_default(),
        before,
        "an unchanged push must not dispatch the platform"
    );
}

/// The published tree is the baseline for every full exporter, for both main
/// configuration and extension. No preceding push is needed to establish it.
#[test]
fn first_full_pull_establishes_the_baseline_for_all_exporters() {
    for provider in ["designer", "ibcmd", "agent"] {
        for extension in [false, true] {
            let project = project(provider, extension);
            pull(&project);
            assert!(!project.sources.join("old.txt").exists());
            assert_push_skips(&project);
        }
    }
}

#[test]
fn full_pull_replaces_a_previous_push_baseline() {
    for provider in ["designer", "ibcmd", "agent"] {
        let project = project(provider, false);
        succeeded(run(&project, &["push"]));
        pull(&project);
        assert_push_skips(&project);
    }
}

#[test]
fn full_pull_replacing_a_source_symlink_records_the_published_directory_identity() {
    let project = project("designer", false);
    let original = project.sources.with_file_name("original-sources");
    fs::rename(&project.sources, &original).expect("move sources");
    std::os::unix::fs::symlink(&original, &project.sources).expect("source symlink");

    pull(&project);

    assert!(!project.sources.is_symlink());
    assert_push_skips(&project);
}

#[test]
fn relative_file_address_uses_the_project_directory_and_matches_absolute_memory() {
    let project = project("designer", false);
    let local = project.config.with_file_name("v8project.local.yaml");
    let absolute = fs::read_to_string(&local).expect("local config");
    let base = project.config.parent().expect("project root").join("ib");
    fs::write(
        &local,
        absolute.replace(&format!("File={}", base.display()), "fIlE = ib"),
    )
    .expect("relative address");
    let caller = temp_workspace();

    succeeded(
        v8_runner_command()
            .current_dir(caller.path())
            .arg("--config")
            .arg(&project.config)
            .arg("--json-message")
            .args(["pull", "--force", "--source-set", "main"])
            .output()
            .expect("pull from another directory"),
    );

    let calls = fs::read_to_string(&project.calls).expect("platform calls");
    assert!(calls.contains(&base.display().to_string()), "{calls}");
    fs::write(&local, absolute).expect("absolute address");
    assert_push_skips(&project);
}

fn git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn snapshot(project: &Project) -> PathBuf {
    project.work.join("infobases/origin/hashes/main.redb")
}

#[test]
fn full_pull_repairs_corrupt_hash_memory() {
    let project = project("designer", false);
    pull(&project);
    let path = snapshot(&project);
    assert!(path.is_file(), "missing snapshot {}", path.display());
    fs::write(&path, "corrupt hash memory").expect("corrupt memory");
    pull(&project);
    assert_push_skips(&project);
}

#[test]
fn git_refusal_preserves_memory_and_a_retry_can_publish() {
    let project = project("designer", false);
    pull(&project);
    let repository = project.sources.parent().expect("project root");
    git(repository, &["init", "-q", "-b", "main"]);
    git(repository, &["add", "sources"]);
    git(repository, &["commit", "-qm", "baseline"]);
    let before = fs::read(snapshot(&project)).expect("snapshot");
    fs::write(project.sources.join("Module.bsl"), "uncommitted local edit").expect("edit");
    // Командная строка просит полную выгрузку только с согласием (`pull --force`); сначала
    // спрашивает систему контроля версий полная выгрузка MCP.
    let refused = support::mcp::call_tool(
        &project.config,
        "dump_config",
        serde_json::json!({ "mode": "FULL" }),
    );
    assert_eq!(
        refused.envelope["ok"], false,
        "publication must refuse local changes: {}",
        refused.envelope
    );
    assert!(
        refused.envelope.to_string().contains("Module.bsl"),
        "the refusal names the local edit: {}",
        refused.envelope
    );
    assert_eq!(
        fs::read(snapshot(&project)).expect("snapshot after refusal"),
        before
    );
    assert_eq!(
        fs::read_to_string(project.sources.join("Module.bsl")).expect("local edit"),
        "uncommitted local edit"
    );
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    assert_push_skips(&project);
}

#[test]
fn a_pull_from_one_base_does_not_mark_another_base_as_loaded() {
    let project = project("designer", false);
    pull(&project);
    let response = succeeded(run(
        &project,
        &["--infobase", "second", "push", "--dry-run"],
    ));
    let steps = response["data"]["steps"].as_array().expect("push steps");
    assert!(
        steps.iter().any(|step| step["mode"] != "skipped"),
        "another base inherited the first baseline: {response}"
    );
    assert_push_skips(&project);
}

#[test]
fn foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials() {
    let project = project("designer", false);
    pull(&project);
    let old_memory = fs::read(snapshot(&project)).expect("memory");
    let local = project.config.with_file_name("v8project.local.yaml");
    let original = fs::read_to_string(&local).expect("local config");
    let old_address = project.config.parent().expect("root").join("ib");
    let new_address = project
        .config
        .parent()
        .expect("root")
        .join("replacement-ib");
    fs::write(
        &local,
        original.replace(
            &format!("File={}", old_address.display()),
            &format!("File={}", new_address.display()),
        ),
    )
    .expect("retarget");
    let before = fs::read_to_string(&project.calls).expect("calls");
    let output = run(&project, &["push"]);
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON refusal");
    let message = json.to_string();
    assert!(message.contains("belongs to"), "{message}");
    assert!(message.contains("pull --force"), "{message}");
    assert!(message.contains("push --full"), "{message}");
    assert!(message.contains("replacement-ib"), "{message}");
    assert!(!message.contains(AGENT_PASSWORD), "{message}");
    assert_eq!(json["data"]["provider_dispatched"], false, "{json}");
    assert_eq!(fs::read_to_string(&project.calls).expect("calls"), before);
    assert_eq!(fs::read(snapshot(&project)).expect("memory"), old_memory);
    pull(&project);
    assert_push_skips(&project);
}

#[test]
fn an_ad_hoc_base_never_uses_the_named_hash_baseline() {
    let project = project("designer", false);
    pull(&project);
    let memory = fs::read(snapshot(&project)).expect("memory");
    let connection = format!(
        "File={}",
        project.config.parent().expect("root").join("ib").display()
    );
    for _ in 0..2 {
        let response = succeeded(run(&project, &["--infobase", &connection, "push"]));
        assert_ne!(
            response["data"]["steps"][0]["mode"], "skipped",
            "{response}"
        );
        assert_eq!(fs::read(snapshot(&project)).expect("named memory"), memory);
    }
    let bases: Vec<_> = fs::read_dir(project.work.join("infobases"))
        .expect("bases")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(bases, [std::ffi::OsString::from("origin")]);
    assert_push_skips(&project);
}

/// Файлы замка выгрузки, лежащие рядом с набором исходников.
fn dump_lock_files(project: &Project) -> Vec<String> {
    let parent = project.sources.parent().expect("source set parent");
    let mut names: Vec<String> = fs::read_dir(parent)
        .expect("source set parent")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with(".dump-") && name.contains(".lock"))
        .collect();
    names.sort();
    names
}

/// Запускает полную выгрузку на заглушке, которая ждёт `release`, и возвращает раннер,
/// когда выгрузка уже идёт и замок выгрузки взят.
fn start_a_blocked_pull(project: &Project, release: &Path) -> RunnerGuard {
    let root = project.sources.parent().expect("project root");
    let started = root.join("dump-started");
    write_shell_script_atomically(&project.binary, &interruptible_stub(&started, release));
    let runner = RunnerGuard(
        v8_runner_command()
            .arg("--config")
            .arg(&project.config)
            .arg("--json-message")
            .args(["pull", "--force", "--source-set", "main"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn pull"),
    );
    assert!(
        wait_for_file(&started, std::time::Duration::from_secs(30)),
        "the dump never started"
    );
    assert!(
        dump_lock_files(project)
            .iter()
            .any(|name| name.ends_with(".lock.system")),
        "the dump runs under its lock: {:?}",
        dump_lock_files(project)
    );
    runner
}

#[test]
fn a_finished_pull_leaves_no_dump_lock_beside_the_source_set() {
    for provider in ["designer", "ibcmd"] {
        for extension in [false, true] {
            let project = project(provider, extension);
            pull(&project);
            assert_eq!(
                dump_lock_files(&project),
                Vec::<String>::new(),
                "{provider}"
            );
        }
    }
}

#[test]
fn an_interrupted_pull_leaves_no_dump_lock_beside_the_source_set() {
    let project = project("designer", false);
    let release = project.work.with_file_name("dump-release");
    let mut runner = start_a_blocked_pull(&project, &release);

    let stopped = terminate_and_wait(&mut runner.0, std::time::Duration::from_secs(30));
    fs::write(&release, "").expect("release a stray dump");

    assert!(stopped, "pull did not stop after SIGTERM");
    assert_eq!(dump_lock_files(&project), Vec::<String>::new());
}

/// После `kill -9` ОС снимает замок, но его файлы остаются. Следующая команда их не
/// пугается: берёт замок как обычно и уносит файлы с собой.
#[test]
fn a_pull_after_a_killed_pull_succeeds_and_removes_the_left_lock_files() {
    let project = project("designer", false);
    let release = project.work.with_file_name("dump-release");
    let mut runner = start_a_blocked_pull(&project, &release);

    runner.0.kill().expect("kill -9 the pull");
    runner.0.wait().expect("killed pull");
    fs::write(&release, "").expect("release the orphaned dump");
    assert_ne!(
        dump_lock_files(&project),
        Vec::<String>::new(),
        "a killed pull cannot remove its lock files"
    );

    write_shell_script_atomically(&project.binary, &dumping_platform(&project.calls));
    pull(&project);

    assert_eq!(dump_lock_files(&project), Vec::<String>::new());
}
