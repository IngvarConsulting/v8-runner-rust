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
        succeeded(run(&project, &["push", "--force"]));
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

/// Фиксирует проект в гите с файлом версий в игноре: выгрузка поверх каталога без
/// незафиксированного идёт без согласия, а каталог вне системы контроля версий она
/// переписать не вправе.
fn commit_project(project: &Project) {
    let root = project.config.parent().expect("project root");
    let ignore = root.join(".gitignore");
    if !ignore.exists() {
        fs::write(&ignore, "ConfigDumpInfo.xml\n").expect("gitignore");
    }
    support::commit_sources(root);
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
    // Память первой базы другой не достаётся: о второй памяти нет, и превью её отправки
    // называет отказ `no_memory`, а не пропуск неизменившегося.
    let output = run(&project, &["--infobase", "second", "push", "--dry-run"]);
    let response: Value = serde_json::from_slice(&output.stdout).expect("JSON refusal");
    assert_eq!(
        response["error"]["code"], "no_memory",
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
    // Чужая память — отсутствие памяти (решение владельца от 06.10.2026): отказ
    // `no_memory` называет её и оба выхода — выгрузку набора и перезапись.
    assert_eq!(json["error"]["code"], "no_memory", "{json}");
    let message = json.to_string();
    assert!(
        message.contains("written for another infobase or source directory"),
        "{message}"
    );
    assert!(message.contains(" pull main`"), "{message}");
    assert!(message.contains(" push --force`"), "{message}");
    assert!(message.contains("replacement-ib"), "{message}");
    assert!(!message.contains(AGENT_PASSWORD), "{message}");
    assert_eq!(json["data"]["provider_dispatched"], false, "{json}");
    assert_eq!(fs::read_to_string(&project.calls).expect("calls"), before);
    assert_eq!(fs::read(snapshot(&project)).expect("memory"), old_memory);
    // Выгрузка набора записывает память для выбранной базы.
    succeeded(run(&project, &["pull", "main", "--force"]));
    assert_push_skips(&project);
}

/// Совет чужой памяти несёт глобальные ключи вызова: `pull main --force` без `--infobase`
/// выгрузил бы базу по умолчанию, а без `--config` из другого каталога — чужой проект.
/// Совет, выполненный буквально из другого каталога, выгружает ту же базу и записывает её
/// память; память базы по умолчанию он не трогает.
#[test]
fn foreign_memory_advice_runs_as_written_against_the_same_base() {
    let project = project("designer", false);
    succeeded(run(
        &project,
        &["--infobase", "second", "pull", "main", "--force"],
    ));
    let second_memory = project.work.join("infobases/second/hashes/main.redb");
    let old_memory = fs::read(&second_memory).expect("memory of the second base");
    assert!(
        !snapshot(&project).exists(),
        "the default base was never used"
    );
    let local = project.config.with_file_name("v8project.local.yaml");
    let original = fs::read_to_string(&local).expect("local config");
    let root = project.config.parent().expect("root");
    let moved = root.join("moved-second-ib");
    fs::write(
        &local,
        original.replace(
            &format!("File={}", root.join("second-ib").display()),
            &format!("File={}", moved.display()),
        ),
    )
    .expect("retarget");

    let output = run(&project, &["--infobase", "second", "push"]);
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON refusal");
    let message = json["error"]["message"].as_str().expect("message");
    assert_eq!(json["error"]["code"], "no_memory", "{json}");
    assert!(message.contains("another infobase"), "{message}");
    // Раннер называет конфиг каноническим путём: на macOS временный `/var/…` — ссылка на
    // `/private/var/…`.
    let config = format!(
        "--config {}",
        fs::canonicalize(&project.config)
            .expect("canonical config")
            .display()
    );
    for tail in ["pull main", "push --force"] {
        let advice = format!("`v8-runner {config} --infobase second {tail}`");
        assert!(message.contains(&advice), "must advise {advice}: {message}");
    }
    // Перезапись названа вместе с потерей: совет не уводит молча в уничтожение.
    assert!(
        message.contains("losing what was changed there"),
        "{message}"
    );
    assert_eq!(fs::read(&second_memory).expect("memory"), old_memory);

    // Совет буквально, оболочкой и из другого каталога.
    let advice = message
        .split('`')
        .find(|part| part.starts_with("v8-runner ") && part.ends_with(" pull main"))
        .expect("pull advice");
    // Выгрузка поверх каталога спрашивает git: каталог под учётом, ей нечего терять.
    support::commit_sources(project.config.parent().expect("root"));
    let binary = support::v8_runner_binary();
    let literal = advice.replacen("v8-runner", &format!("'{}'", binary.display()), 1);
    let elsewhere = tempfile::tempdir().expect("another directory");
    let before = fs::read_to_string(&project.calls).expect("calls");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&literal)
        .current_dir(elsewhere.path())
        .output()
        .expect("run the advice");
    assert!(
        output.status.success(),
        "{literal}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(&project.calls).expect("calls");
    let dispatched = &calls[before.len()..];
    assert!(
        dispatched.contains(&moved.display().to_string()),
        "the advice must dump the selected base: {dispatched}"
    );
    assert!(
        !snapshot(&project).exists(),
        "the advice must not touch the default base"
    );
    // Поддельная платформа о поколении не отвечает, а выгрузка поверх каталога хеш-памяти не
    // пишет; память о выбранной базе записывает полная выгрузка.
    succeeded(run(
        &project,
        &["--infobase", "second", "pull", "main", "--force"],
    ));
    assert_ne!(fs::read(&second_memory).expect("memory"), old_memory);
    succeeded(run(&project, &["--infobase", "second", "push"]));
}

/// База, названная строкой соединения, помнится по строке: первая отправка грузит, вторая
/// с той же строкой продолжает с памятью и пропускает неизменившееся. Память именованной
/// базы при этом не трогается, а каталог памяти по строке с её именем не совпадает.
#[test]
fn an_ad_hoc_base_is_remembered_by_its_connection_string() {
    let project = project("designer", false);
    pull(&project);
    let memory = fs::read(snapshot(&project)).expect("memory");
    let connection = format!(
        "File={}",
        project.config.parent().expect("root").join("ib").display()
    );
    let first = succeeded(run(
        &project,
        &["--infobase", &connection, "push", "--force"],
    ));
    assert_ne!(first["data"]["steps"][0]["mode"], "skipped", "{first}");
    assert_eq!(fs::read(snapshot(&project)).expect("named memory"), memory);
    let again = succeeded(run(&project, &["--infobase", &connection, "push"]));
    assert_eq!(again["data"]["steps"][0]["mode"], "skipped", "{again}");
    let mut bases: Vec<_> = fs::read_dir(project.work.join("infobases"))
        .expect("bases")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    bases.sort();
    assert_eq!(bases.len(), 2, "{bases:?}");
    assert!(bases[0].starts_with('@'), "{bases:?}");
    assert_eq!(bases[1], "origin");
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

/// Поддельная платформа, которая, как настоящая, пишет файл версий: выгрузка — в каталог
/// выгрузки, загрузка Конфигуратора с `-updateConfigDumpInfo` — в каталог загрузки;
/// `ibcmd config import` его не пишет. Выгрузка по изменившемуся и загрузка, пишущая файл
/// версий, записывают в `seen`, какой файл версий застали в каталоге. С файлом `fail` рядом вызов, успев записать
/// файл версий, кончается сбоем.
fn version_writing_platform(root: &Path) -> String {
    let calls = root.join("calls.log");
    let counter = root.join("counter");
    let seen = root.join("seen.log");
    let fail = root.join("fail");
    format!(
        r#"printf '%s\n' "$*" >> '{calls}'
case " $* " in *" -getChanges "*|*" export status "*) exit 0;; esac
target=''
load=''
update=''
writes=''
exports=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/DumpConfigToFiles' ]; then target="$arg"; fi
  if [ "$previous" = '/LoadConfigFromFiles' ]; then load="$arg"; fi
  if [ "$arg" = '-update' ] || [ "$arg" = '--sync' ]; then update=1; fi
  if [ "$arg" = '-updateConfigDumpInfo' ]; then writes=1; fi
  if [ "$previous" = 'config' ] && [ "$arg" = 'export' ]; then exports=1; fi
  previous="$arg"
done
if [ -n "$exports" ]; then target="$previous"; fi
n=$(( $(cat '{counter}' 2>/dev/null || echo 0) + 1 ))
printf '%s\n' "$n" > '{counter}'
if [ -n "$target" ]; then
  mkdir -p "$target"
  if [ -n "$update" ]; then cat "$target/ConfigDumpInfo.xml" >> '{seen}'; fi
  printf '<Configuration/>\n' > "$target/Configuration.xml"
  printf '<ConfigDumpInfo version="2.17" dump="%s"/>\n' "$n" > "$target/ConfigDumpInfo.xml"
fi
if [ -n "$load" ] && [ -n "$writes" ]; then
  cat "$load/ConfigDumpInfo.xml" >> '{seen}'
  printf '<ConfigDumpInfo version="2.17" load="%s"/>\n' "$n" > "$load/ConfigDumpInfo.xml"
fi
if [ -f '{fail}' ]; then exit 1; fi
exit 0"#,
        calls = calls.display(),
        counter = counter.display(),
        seen = seen.display(),
        fail = fail.display(),
    )
}

fn version_project(provider: &str) -> Project {
    let project = project(provider, false);
    write_shell_script_atomically(
        &project.binary,
        &version_writing_platform(project.config.parent().expect("project root")),
    );
    project
}

fn runner_copy(project: &Project) -> PathBuf {
    project
        .work
        .join("infobases/origin/dump-info/main/ConfigDumpInfo.xml")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Последний файл версий, от которого выгрузка по изменившемуся считала разницу.
fn last_seen(project: &Project) -> String {
    let seen = read(&project.config.with_file_name("seen.log"));
    seen.lines()
        .last()
        .expect("an incremental dump ran")
        .to_owned()
}

fn fail_platform(project: &Project, fail: bool) {
    let flag = project.config.with_file_name("fail");
    if fail {
        fs::write(flag, "").expect("fail flag");
    } else {
        fs::remove_file(flag).expect("clear fail flag");
    }
}

#[test]
fn a_foreign_version_file_between_commands_does_not_reach_the_dump() {
    for provider in ["designer", "ibcmd"] {
        let project = version_project(provider);
        succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
        let version_file = project.sources.join("ConfigDumpInfo.xml");
        assert_eq!(
            read(&runner_copy(&project)),
            read(&version_file),
            "{provider}"
        );
        let ours = read(&version_file);
        commit_project(&project);

        fs::write(&version_file, "<ConfigDumpInfo foreign=\"1\"/>\n").expect("foreign write");
        succeeded(run(&project, &["pull", "--source-set", "main"]));

        assert_eq!(format!("{}\n", last_seen(&project)), ours, "{provider}");
        assert!(read(&version_file).contains("dump="), "{provider}");
        assert_ne!(read(&version_file), ours, "{provider}");
        assert_eq!(
            read(&runner_copy(&project)),
            read(&version_file),
            "{provider}"
        );
    }
}

#[test]
fn a_failed_pull_does_not_change_the_runner_copy() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let ours = read(&runner_copy(&project));
    commit_project(&project);

    fail_platform(&project, true);
    for args in [
        &["pull", "--source-set", "main"][..],
        &["pull", "--force", "--source-set", "main"][..],
    ] {
        assert!(!run(&project, args).status.success(), "{args:?}");
        assert_eq!(read(&runner_copy(&project)), ours, "{args:?}");
    }
    // Сбой выгрузки по изменившемуся успел переписать файл в каталоге; следующая выгрузка
    // всё равно считает разницу от копии раннера.
    assert_ne!(read(&project.sources.join("ConfigDumpInfo.xml")), ours);

    fail_platform(&project, false);
    succeeded(run(&project, &["pull", "--source-set", "main"]));
    assert_eq!(format!("{}\n", last_seen(&project)), ours);
}

#[test]
fn a_push_refreshes_the_runner_copy() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let version_file = project.sources.join("ConfigDumpInfo.xml");
    let ours = read(&version_file);
    fs::write(&version_file, "<ConfigDumpInfo foreign=\"1\"/>\n").expect("foreign write");
    fs::write(
        project.sources.join("Module.bsl"),
        "Procedure Edited()\nEndProcedure\n",
    )
    .expect("edit");

    succeeded(run(&project, &["push"]));

    // Загрузка обновляет файл раннера, а не подменённый.
    assert_eq!(format!("{}\n", last_seen(&project)), ours);
    let loaded = read(&version_file);
    assert!(loaded.contains("load="), "{loaded}");
    assert_eq!(read(&runner_copy(&project)), loaded);

    fs::write(&version_file, "<ConfigDumpInfo foreign=\"2\"/>\n").expect("foreign write");
    commit_project(&project);
    succeeded(run(&project, &["pull", "--source-set", "main"]));
    assert_eq!(format!("{}\n", last_seen(&project)), loaded);
}

#[test]
fn a_push_that_writes_no_version_file_keeps_the_runner_copy() {
    let project = version_project("ibcmd");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let ours = read(&runner_copy(&project));
    let version_file = project.sources.join("ConfigDumpInfo.xml");
    fs::write(&version_file, "<ConfigDumpInfo foreign=\"1\"/>\n").expect("foreign write");
    fs::write(
        project.sources.join("Module.bsl"),
        "Procedure Edited()\nEndProcedure\n",
    )
    .expect("edit");

    succeeded(run(&project, &["push"]));

    assert_eq!(read(&runner_copy(&project)), ours);
    commit_project(&project);
    succeeded(run(&project, &["pull", "--source-set", "main"]));
    assert_eq!(format!("{}\n", last_seen(&project)), ours);
}

#[test]
fn a_failed_copy_write_is_a_warning_not_a_refusal() {
    let project = version_project("designer");
    let memory = project.work.join("infobases/origin");
    fs::create_dir_all(&memory).expect("memory dir");
    fs::write(memory.join("dump-info"), "not a directory").expect("block the copy dir");

    let response = succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));

    let message = response["data"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("ConfigDumpInfo.xml for source-set 'main' was not updated"),
        "{response}"
    );
    assert!(project.sources.join("ConfigDumpInfo.xml").is_file());
}

#[test]
fn a_designer_partial_pull_leaves_the_runner_copy_alone() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let ours = read(&runner_copy(&project));
    commit_project(&project);

    succeeded(run(
        &project,
        &["pull", "--source-set", "main", "--object", "Catalog:Items"],
    ));

    assert_ne!(read(&project.sources.join("ConfigDumpInfo.xml")), ours);
    assert_eq!(read(&runner_copy(&project)), ours);
}

#[test]
fn an_ibcmd_partial_pull_dumps_from_the_runner_copy() {
    let project = version_project("ibcmd");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let ours = read(&runner_copy(&project));
    commit_project(&project);
    let version_file = project.sources.join("ConfigDumpInfo.xml");
    fs::write(&version_file, "<ConfigDumpInfo foreign=\"1\"/>\n").expect("foreign write");

    succeeded(run(
        &project,
        &["pull", "--source-set", "main", "--object", "Catalog:Items"],
    ));

    assert_eq!(format!("{}\n", last_seen(&project)), ours);
    assert_eq!(read(&runner_copy(&project)), read(&version_file));
}

/// Агент загружает через ссылку на каталог набора: перед загрузкой там лежит файл раннера,
/// а записанный агентом становится копией.
#[test]
fn an_agent_push_loads_over_the_runner_copy_and_records_the_new_one() {
    let project = project("agent", false);
    let version_file = project.sources.join("ConfigDumpInfo.xml");
    succeeded(run(&project, &["push", "--force"]));
    let ours = read(&version_file);
    assert!(ours.contains("agent-load="), "{ours}");
    assert_eq!(read(&runner_copy(&project)), ours);

    fs::write(&version_file, "<ConfigDumpInfo foreign=\"1\"/>\n").expect("foreign write");
    fs::write(
        project.sources.join("Module.bsl"),
        "Procedure Edited()\nEndProcedure\n",
    )
    .expect("edit");
    succeeded(run(&project, &["push"]));

    let seen = read(&project.calls.with_extension("version-files"));
    let seen = seen
        .lines()
        .next_back()
        .expect("the agent loaded with --update-config-dump-info");
    assert_eq!(format!("{seen}\n"), ours);
    assert_eq!(read(&runner_copy(&project)), read(&version_file));
    assert_ne!(read(&version_file), ours);
}

/// Временный файл замены, брошенный снятым процессом, не числится незафиксированной
/// работой: выгрузка убирает его раньше, чем сторож спрашивает гит.
#[test]
fn a_left_temporary_version_file_does_not_stop_a_full_pull() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let repository = project.sources.parent().expect("project root");
    fs::write(repository.join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    git(repository, &["init", "-q", "-b", "main"]);
    git(repository, &["add", ".gitignore", "sources"]);
    git(repository, &["commit", "-qm", "baseline"]);
    let left = project.sources.join("ConfigDumpInfo.xml.candidate-x1");
    fs::write(&left, "half written").expect("left candidate");

    let response = support::mcp::call_tool(
        &project.config,
        "dump_config",
        serde_json::json!({ "mode": "FULL" }),
    );

    assert_eq!(response.envelope["ok"], true, "{}", response.envelope);
    assert!(!left.exists());
}

#[test]
fn a_failed_push_does_not_change_the_runner_copy() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    let ours = read(&runner_copy(&project));
    fs::write(
        project.sources.join("Module.bsl"),
        "Procedure Edited()\nEndProcedure\n",
    )
    .expect("edit");

    fail_platform(&project, true);
    assert!(!run(&project, &["push"]).status.success());

    assert_ne!(read(&project.sources.join("ConfigDumpInfo.xml")), ours);
    assert_eq!(read(&runner_copy(&project)), ours);
}

/// Сверяется только сам файл версий: правка исходников выгрузку полной не делает.
#[test]
fn a_source_edit_keeps_the_pull_incremental() {
    let project = version_project("designer");
    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    fs::write(
        project.sources.join("Module.bsl"),
        "Procedure Edited()\nEndProcedure\n",
    )
    .expect("edit");
    // Правка зафиксирована: незафиксированную выгрузка поверх каталога не переписывает.
    commit_project(&project);

    succeeded(run(&project, &["pull", "--source-set", "main"]));

    let calls = read(&project.calls);
    let last = calls.lines().next_back().expect("a dump ran");
    assert!(last.contains("/DumpConfigToFiles"), "{last}");
    assert!(last.contains("-update"), "{last}");
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Полная выгрузка поверх каталога без файла версий переписывает файлы человека, как и
/// замена каталога: незакоммиченная правка отслеживаемого файла останавливает её до
/// платформы, отказ называет файл и выход `pull main --force`, файл цел. Выход работает:
/// с согласием выгрузка идёт полной.
#[test]
fn a_full_dump_over_the_directory_asks_the_replacement_guard() {
    for provider in ["designer", "ibcmd", "agent"] {
        let project = project(provider, false);
        let repository = project.sources.parent().expect("project root");
        fs::write(repository.join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
        git(repository, &["init", "-q", "-b", "main"]);
        git(repository, &["add", ".gitignore", "sources"]);
        git(repository, &["commit", "-qm", "baseline"]);
        let edited = project.sources.join("old.txt");
        fs::write(&edited, "an edit nobody committed").expect("edit");
        assert!(!project.sources.join("ConfigDumpInfo.xml").exists());

        let output = run(&project, &["pull", "--source-set", "main"]);

        assert_eq!(
            output.status.code(),
            Some(2),
            "{provider}: {}",
            output_text(&output)
        );
        let text = output_text(&output);
        assert!(text.contains("refusing to overwrite"), "{provider}: {text}");
        assert!(text.contains("old.txt"), "{provider}: {text}");
        assert!(text.contains("pull main --force"), "{provider}: {text}");
        assert_eq!(read(&edited), "an edit nobody committed", "{provider}");
        let calls = fs::read_to_string(&project.calls).unwrap_or_default();
        assert!(
            !calls.contains("DumpConfigToFiles")
                && !calls.contains("export")
                && !calls.contains("dump-config-to-files"),
            "{provider}: the platform must not start: {calls}"
        );

        let response = succeeded(run(&project, &["pull", "main", "--force"]));
        assert_eq!(response["data"]["mode"], "FULL", "{provider}: {response}");
    }
}

/// Превью называет тот режим, который выполнит выгрузка: без файла версий — полный и
/// причину; подменённый файл при своей копии раннера уступит ей, и выгрузка останется по
/// изменившемуся. Платформа не запускается.
#[test]
fn a_preview_names_the_mode_the_pull_would_run() {
    let project = version_project("designer");
    let preview = |project: &Project| {
        let calls_before = fs::read_to_string(&project.calls).unwrap_or_default();
        let response = succeeded(run(project, &["pull", "--source-set", "main", "--dry-run"]));
        assert_eq!(
            fs::read_to_string(&project.calls).unwrap_or_default(),
            calls_before,
            "the preview must not start the platform"
        );
        response
    };

    let missing = preview(&project);
    assert_eq!(missing["data"]["mode"], "FULL", "{missing}");
    assert!(
        missing["data"]["message"].as_str().is_some_and(|message| {
            message.contains("no version file ConfigDumpInfo.xml")
                && message.contains("would run full instead of incremental")
        }),
        "{missing}"
    );

    succeeded(run(&project, &["pull", "--force", "--source-set", "main"]));
    commit_project(&project);
    let version_file = project.sources.join("ConfigDumpInfo.xml");
    fs::write(&version_file, "<ConfigDumpInfo/>\n").expect("a replaced version file");
    let restored = preview(&project);
    assert_eq!(restored["data"]["mode"], "INCREMENTAL", "{restored}");
    assert_eq!(read(&version_file), "<ConfigDumpInfo/>\n");
    let response = succeeded(run(&project, &["pull", "--source-set", "main"]));
    assert_eq!(response["data"]["mode"], "INCREMENTAL", "{response}");

    fs::remove_file(&version_file).expect("a lost version file");
    let lost = preview(&project);
    assert_eq!(lost["data"]["mode"], "FULL", "{lost}");
    let response = succeeded(run(&project, &["pull", "--source-set", "main"]));
    assert_eq!(response["data"]["mode"], "FULL", "{response}");
}

/// Полная выгрузка поверх каталога без файла версий лишнего не удаляет и хеш-память не
/// пишет: каталог с файлами, которых нет в базе, базу не описывает. Ответ называет это и
/// совет `pull <SET> --force` для полного выравнивания.
#[test]
fn a_full_dump_over_the_directory_names_the_memory_it_does_not_write() {
    let project = project("designer", false);
    commit_project(&project);
    assert!(!project.sources.join("ConfigDumpInfo.xml").exists());

    let response = succeeded(run(&project, &["pull", "--source-set", "main"]));

    assert_eq!(response["data"]["mode"], "FULL", "{response}");
    let message = response["data"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("files the base does not have stay in the directory")
            && message.contains("hash memory is not updated")
            && message.contains("pull main --force"),
        "{response}"
    );
    assert_eq!(
        read(&project.sources.join("old.txt")),
        "local contents before pull"
    );
    assert!(
        !project
            .work
            .join("infobases/origin/hashes/main.redb")
            .exists(),
        "a dump over the directory records no hashes"
    );
}
