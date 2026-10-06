//! Память о базе и её поколение перед обменом (#215).
//!
//! Поддельный Конфигуратор отвечает `/GetConfigGenerationID` токеном из файла `token` рядом
//! с собой; тест меняет его, как меняла бы базу правка в Конфигураторе. Загрузка файла версий
//! не пишет — так видно восстановление `-configDumpInfoOnly`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";

/// Поддельный Конфигуратор: журнал вызовов, токен поколения из файла, выгрузка.
fn platform(root: &Path) -> String {
    format!(
        r#"printf '%s\n' "$*" >> '{calls}'
out=''
target=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  if [ "$previous" = '/DumpConfigToFiles' ]; then target="$arg"; fi
  previous="$arg"
done
case "$*" in
  *'/GetConfigGenerationID'*)
    if [ -f '{token}' ]; then cat '{token}' > "$out"; fi
    exit 0 ;;
  *'-configDumpInfoOnly'*)
    printf '<ConfigDumpInfo version="2.17" info-only="1"/>\n' > "$target/ConfigDumpInfo.xml"
    if [ -f '{drift}' ]; then cp '{drift}' '{token}'; fi
    exit 0 ;;
  *'/DumpConfigToFiles'*)
    mkdir -p "$target"
    printf '<Configuration/>\n' > "$target/Configuration.xml"
    printf '<ConfigDumpInfo version="2.17"/>\n' > "$target/ConfigDumpInfo.xml"
    if [ -f '{drift}' ]; then cp '{drift}' '{token}'; fi
    exit 0 ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
        token = root.join("token").display(),
        drift = root.join("drift").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
    sources: PathBuf,
}

impl Project {
    /// Проект с одним набором `main` и файловой базой `connection` (по умолчанию `File=ib`).
    fn new(connection: &str) -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
        write_shell_script(&root.join("1cv8"), &platform(root));
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                root.join("1cv8").display()
            ),
        )
        .expect("config");
        fs::write(
            root.join("v8project.local.yaml"),
            format!("infobases:\n  origin:\n    connection: '{connection}'\n"),
        )
        .expect("local layer");
        Self {
            dir,
            config,
            sources,
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Поколение, которым база ответит дальше.
    fn base_generation(&self, token: &str) {
        fs::write(self.root().join("token"), format!("{token}\r\n")).expect("token");
    }

    fn run(&self, args: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run CLI")
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root().join("calls.log")).unwrap_or_default()
    }

    fn forget_calls(&self) {
        let _ = fs::remove_file(self.root().join("calls.log"));
    }

    fn edit(&self) {
        fs::write(
            self.sources.join("Module.bsl"),
            "Procedure A()\n// edited\nEndProcedure\n",
        )
        .expect("edit");
    }

    fn ledger(&self) -> Value {
        let text = fs::read_to_string(
            self.root()
                .join("work")
                .join("infobases")
                .join("origin")
                .join("generation.json"),
        )
        .expect("generation ledger");
        serde_json::from_str(&text).expect("ledger json")
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn succeeded(output: &Output) -> Value {
    let payload = envelope(output);
    assert!(output.status.success(), "{payload}");
    payload
}

/// Без памяти о базе `push` отказывает до платформы и называет оба выхода: `pull` следующим
/// шагом и `push --force` текстом; пустую базу он не различает.
#[test]
fn a_push_without_memory_of_the_base_is_refused_with_both_ways_out() {
    let project = Project::new("File=ib");
    project.base_generation("0000000000000000000000000000000000000000");

    let refused = project.run(&["push"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["kind"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    assert_eq!(payload["error"]["next"]["source_set"], "main", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("pull main`"), "{message}");
    assert!(message.contains("push --force`"), "{message}");
    assert!(
        project.calls().is_empty(),
        "the platform does not start before the refusal: {}",
        project.calls()
    );
}

/// `push --force` грузит без памяти, поколение записывается с инструментом, и следующая
/// отправка уже не отказывает.
#[test]
fn a_push_force_loads_without_memory_and_remembers_the_generation() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);

    let forced = succeeded(&project.run(&["push", "--force"]));
    assert_eq!(forced["data"]["steps"][0]["mode"], "full", "{forced}");
    let record = &project.ledger()["main"];
    assert_eq!(record["token"], FIRST, "{record}");
    assert_eq!(record["tool"], "designer", "{record}");
    assert_eq!(record["after"], "build", "{record}");

    project.edit();
    succeeded(&project.run(&["push"]));
}

/// База ушла вперёд записанного поколения: отказ `non_fast_forward` до загрузки, с обоими
/// поколениями и `pull` следующим шагом; `push --force` её перезаписывает.
#[test]
fn a_push_into_a_base_that_moved_ahead_is_refused_before_the_load() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);
    project.edit();
    project.forget_calls();

    let refused = project.run(&["push"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["base_generation"], SECOND, "{payload}");
    assert_eq!(payload["error"]["local_generation"], FIRST, "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    assert_eq!(payload["error"]["next"]["source_set"], "main", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("push main --force`"), "{message}");
    assert!(
        !message.contains("another working copy"),
        "a file base is protected by its marker: {message}"
    );
    assert!(
        !project.calls().contains("/LoadConfigFromFiles"),
        "nothing is loaded: {}",
        project.calls()
    );

    succeeded(&project.run(&["push", "--force"]));
    assert_eq!(project.ledger()["main"]["token"], SECOND);
}

/// У базы в кластере отказ говорит ещё, что базу могла изменить другая рабочая копия.
#[test]
fn a_server_base_that_moved_ahead_may_have_been_changed_by_another_copy() {
    let project = Project::new("Srvr=cluster;Ref=dev");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);
    project.edit();

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("another working copy"), "{message}");
}

/// Взявшая базу без метки копия до первой удачной отправки выгрузку не предлагает: выход —
/// `push --force`. После него отказ снова предлагает `pull`.
#[test]
fn a_new_owner_is_offered_no_pull_until_its_first_push() {
    let project = Project::new("File=ib");
    let base = project.root().join("ib");
    fs::create_dir_all(&base).expect("base");
    fs::write(base.join("1Cv8.1CD"), "database").expect("base file");
    support::memory::remember_base(
        &project.root().join("work"),
        "origin",
        support::memory::Base::File(&base),
        &[support::memory::Set::configuration(
            "main",
            &project.sources,
        )],
    );
    // Память записана Конфигуратором с нулевым токеном, а база ответит другим.
    project.base_generation(FIRST);

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "push", "{payload}");
    assert_eq!(payload["error"]["next"]["keys"]["--force"], "", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(!message.contains("pull"), "{message}");
    assert!(message.contains("took the infobase over"), "{message}");

    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);
    project.edit();
    let payload = envelope(&project.run(&["push"]));
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
}

/// Базу правили во время выгрузки: ответ это называет, а память о поколении не обновляется,
/// и следующая отправка видит расхождение.
#[test]
fn a_base_changed_during_a_pull_is_named_and_not_remembered() {
    let project = Project::new("File=ib");
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    fs::write(project.root().join("drift"), format!("{SECOND}\n")).expect("drift");

    let pulled = succeeded(&project.run(&["pull", "main", "--force"]));

    let message = pulled["data"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("changed while source-set 'main' was being dumped"),
        "{pulled}"
    );
    assert_eq!(project.ledger()["main"]["token"], FIRST);
    project.edit();
    let payload = envelope(&project.run(&["push"]));
    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
}

/// Выгрузка, до и после которой поколение то же, записывает его: следующая отправка проходит.
#[test]
fn a_pull_records_the_generation_it_saw_before_and_after() {
    let project = Project::new("File=ib");
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);

    succeeded(&project.run(&["pull", "main", "--force"]));

    let record = &project.ledger()["main"];
    assert_eq!(record["token"], SECOND, "{record}");
    assert_eq!(record["after"], "dump", "{record}");
    project.edit();
    succeeded(&project.run(&["push"]));
}

/// Потерянный файл версий восстанавливается одной выгрузкой `-configDumpInfoOnly` сразу после
/// полной отправки, когда поколение до и после неё одно и то же.
#[test]
fn a_lost_version_file_is_dumped_alone_after_a_full_push() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);

    let pushed = succeeded(&project.run(&["push", "--force"]));

    assert!(
        project.calls().contains("-configDumpInfoOnly"),
        "{}",
        project.calls()
    );
    assert!(project.sources.join("ConfigDumpInfo.xml").is_file());
    let message = pushed["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("dumped alone"), "{message}");
}

/// Поколение изменилось во время выгрузки файла версий — файл не остаётся: совпадение
/// каталога и базы не доказано.
#[test]
fn a_version_file_dumped_while_the_base_changed_is_removed() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    fs::write(project.root().join("drift"), format!("{SECOND}\n")).expect("drift");

    let pushed = succeeded(&project.run(&["push", "--force"]));

    assert!(!project.sources.join("ConfigDumpInfo.xml").exists());
    let message = pushed["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("was not restored"), "{message}");
}

/// Без ответа о поколении файл версий отдельно не выгружается.
#[test]
fn without_an_answer_the_version_file_is_not_dumped_alone() {
    let project = Project::new("File=ib");

    succeeded(&project.run(&["push", "--force"]));

    assert!(
        !project.calls().contains("-configDumpInfoOnly"),
        "{}",
        project.calls()
    );
    assert!(!project.sources.join("ConfigDumpInfo.xml").exists());
}

/// `ibcmd` отвечает поколением в последней непустой строке stdout после приглашения ввести
/// пароль; предупреждения СУБД в stderr ответом не являются. Расхождение — тот же отказ.
#[test]
fn ibcmd_reads_the_generation_from_its_last_stdout_line() {
    let project = Project::new("File=ib");
    let root = project.root().to_path_buf();
    write_shell_script(
        &root.join("ibcmd"),
        &format!(
            r#"printf '%s\n' "$*" >> '{calls}'
case "$*" in
  *'generation-id'*)
    printf 'ПРЕДУПРЕЖДЕНИЕ:  нет незавершённой транзакции\n' >&2
    printf 'Введите пароль для подключения к базе данных: \n'
    cat '{token}'
    exit 0 ;;
esac
exit 0"#,
            calls = root.join("calls.log").display(),
            token = root.join("token").display(),
        ),
    );
    fs::write(
        &project.config,
        format!(
            "workPath: work\nformat: DESIGNER\nproviders:\n  build: ibcmd\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
            root.join("ibcmd").display()
        ),
    )
    .expect("config");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    let record = &project.ledger()["main"];
    assert_eq!(record["token"], FIRST, "{record}");
    assert_eq!(record["tool"], "ibcmd", "{record}");

    project.base_generation(SECOND);
    project.edit();
    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["base_generation"], SECOND, "{payload}");
}

/// Поколение, записанное Конфигуратором, `ibcmd` не сверяет: токены разных инструментов
/// несравнимы, и расхождения они не дают.
#[test]
fn a_generation_of_another_tool_does_not_refuse_a_push() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    let root = project.root().to_path_buf();
    write_shell_script(
        &root.join("ibcmd"),
        &format!("case \"$*\" in *generation-id*) printf '{SECOND}\\n' ;; esac\nexit 0"),
    );
    fs::write(
        &project.config,
        fs::read_to_string(&project.config)
            .expect("config")
            .replace(
                "format: DESIGNER\n",
                "format: DESIGNER\nproviders:\n  build: ibcmd\n",
            )
            .replace(
                &root.join("1cv8").display().to_string(),
                &root.join("ibcmd").display().to_string(),
            ),
    )
    .expect("config");
    project.edit();

    succeeded(&project.run(&["push"]));
    assert_eq!(project.ledger()["main"]["tool"], "ibcmd");
}
