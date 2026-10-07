//! Память о базе и её поколение перед обменом (#215).
//!
//! Поддельный Конфигуратор отвечает `/GetConfigGenerationID` токеном из файла `token` рядом
//! с собой (у расширения `ext` — из `token-ext`, если он есть); тест меняет его, как меняла
//! бы базу правка в Конфигураторе. Загрузка файла версий не пишет — так видно восстановление
//! `-configDumpInfoOnly`; при файле `fail` она падает, сдвинув поколение на `drift`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{json, Value};
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
    file='{token}'
    case "$*" in *'-Extension ext'*) if [ -f '{token}-ext' ]; then file='{token}-ext'; fi ;; esac
    if [ -f "$file" ]; then cat "$file" > "$out"; fi
    exit 0 ;;
  *'CREATEINFOBASE'*)
    mkdir -p '{base}'
    : > '{base}/1Cv8.1CD'
    exit 0 ;;
  *'/LoadConfigFromFiles'*)
    if [ -f '{fail}' ]; then
      if [ -f '{drift}' ]; then cp '{drift}' '{token}'; fi
      exit 1
    fi ;;
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
        fail = root.join("fail").display(),
        base = root.join("ib").display(),
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
                "workPath: work\nformat: DESIGNER\n{designer_leads}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                root.join("1cv8").display(),
 designer_leads = support::DESIGNER_LEADS,
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

    /// Второй набор — расширение `ext` в каталоге `ext`.
    fn with_extension(self) -> Self {
        let ext = self.root().join("ext");
        fs::create_dir_all(&ext).expect("extension sources");
        fs::write(ext.join("Configuration.xml"), "<Configuration/>\n").expect("extension");
        fs::write(ext.join("Module.bsl"), "Procedure B()\nEndProcedure\n").expect("module");
        fs::write(
            &self.config,
            fs::read_to_string(&self.config).expect("config").replace(
                "    path: sources\n",
                "    path: sources\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
            ),
        )
        .expect("config");
        self
    }

    /// Поколение, которым ответит расширение `ext`.
    fn extension_generation(&self, token: &str) {
        fs::write(self.root().join("token-ext"), format!("{token}\r\n")).expect("token");
    }

    fn edit_extension(&self) {
        fs::write(
            self.root().join("ext").join("Module.bsl"),
            "Procedure B()\n// edited\nEndProcedure\n",
        )
        .expect("edit");
    }

    /// Файл версий принадлежит базе и в git не хранится.
    fn ignore_version_files(&self) {
        fs::write(self.root().join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    }

    fn ledger_file(&self) -> PathBuf {
        self.root()
            .join("work")
            .join("infobases")
            .join("origin")
            .join("generation.json")
    }

    fn ledger(&self) -> Value {
        let text = fs::read_to_string(self.ledger_file()).expect("generation ledger");
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

/// Копии базы (`infobase create --from`) до первой отправки ни один ответ не предлагает
/// `pull`: отказ называет только `push --force` и то, что база — копия; первая отправка
/// снимает признак, и выгрузка снова становится выходом.
#[test]
fn a_copied_base_offers_no_pull_before_its_first_push() {
    let project = Project::new("File=ib");
    let base = project.root().join("ib");
    fs::create_dir_all(&base).expect("base");
    fs::write(base.join("1Cv8.1CD"), "database").expect("base file");
    let memory = project.root().join("work").join("infobases").join("origin");
    support::memory::remember_base(
        &project.root().join("work"),
        "origin",
        support::memory::Base::File(&base),
        &[support::memory::Set::configuration(
            "main",
            &project.sources,
        )],
    );
    // Запись поколения сделана выгрузкой после копии, признак копии остался: база ответит
    // другим токеном.
    fs::write(
        memory.join("copied-from.json"),
        r#"{"source":"upstream","snapshot":"/work/copies/upstream.dt","since":"2026-10-07T00:00:00Z","generation":null}"#,
    )
    .expect("copy mark");
    project.base_generation(FIRST);

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "push", "{payload}");
    assert_eq!(payload["error"]["next"]["keys"]["--force"], "", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(!message.contains("pull"), "{message}");
    assert!(
        message.contains("is a copy of the infobase 'upstream'"),
        "{message}"
    );

    succeeded(&project.run(&["push", "--force"]));
    assert!(!memory.join("copied-from.json").exists());
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

/// `pull --all` выгружает каждый набор как `pull <SET>` и так же записывает поколение, после
/// чего обычная отправка не отказывает.
#[test]
fn a_pull_all_records_the_generation_of_each_set() {
    let project = Project::new("File=ib");
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);

    succeeded(&project.run(&["pull", "--all", "--force"]));

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
            .replace("  push: designer\n", "  push: ibcmd\n")
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

/// Первый `pull` без памяти о базе, во время которого базу правили: ответ это называет, а
/// записывается поколение до выгрузки — и следующий `push` отказывает `non_fast_forward`,
/// а не проходит молча по хеш-памяти, которую эта выгрузка записала.
#[test]
fn a_first_pull_that_saw_the_base_change_leaves_the_next_push_refused() {
    let project = Project::new("File=ib");
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    fs::write(project.root().join("drift"), format!("{SECOND}\n")).expect("drift");

    let pulled = succeeded(&project.run(&["pull", "main", "--force"]));

    let message = pulled["data"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("was being dumped"), "{pulled}");
    assert!(message.contains("non-fast-forward"), "{pulled}");
    assert_eq!(project.ledger()["main"]["token"], FIRST);
    fs::remove_file(project.root().join("drift")).expect("drift");
    project.edit();
    project.forget_calls();

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["base_generation"], SECOND, "{payload}");
    assert_eq!(payload["error"]["local_generation"], FIRST, "{payload}");
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
}

/// Память, записанная для другой базы под тем же именем, памятью не считается и у полной
/// загрузки: `push --full` и `build_project` с `full_rebuild` отказывают `no_memory`, а не
/// грузят молча; выходы называют CLI-команду `push --force`, в том числе у MCP.
#[test]
fn a_full_push_with_memory_of_another_base_is_refused_as_no_memory() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    let local = project.root().join("v8project.local.yaml");
    fs::write(
        &local,
        fs::read_to_string(&local)
            .expect("local layer")
            .replace("File=ib", "File=replacement-ib"),
    )
    .expect("retarget");
    project.forget_calls();

    let refused = project.run(&["push", "--full"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("push --force`"), "{message}");
    assert!(
        project.calls().is_empty(),
        "nothing is loaded: {}",
        project.calls()
    );

    let answer = support::mcp::call_tool(
        &project.config,
        "build_project",
        json!({"full_rebuild": true}),
    );

    assert!(answer.is_error, "{}", answer.envelope);
    let payload = &answer.envelope;
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("`v8-runner"), "{message}");
    assert!(message.contains("push --force`"), "{message}");
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// `push --full` проходит сверку поколения, как обычная отправка: база ушла вперёд —
/// отказ `non_fast_forward` до загрузки. Обходит её только `--force`.
#[test]
fn a_full_push_into_a_base_that_moved_ahead_is_refused() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);
    project.forget_calls();

    let payload = envelope(&project.run(&["push", "--full"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["base_generation"], SECOND, "{payload}");
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
}

/// Превью отправки без памяти о базе называет тот же отказ `no_memory`, что и прогон, и
/// платформу для этого не запускает; с памятью превью проходит.
#[test]
fn a_preview_of_a_push_without_memory_names_the_refusal() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);

    let refused = project.run(&["push", "--dry-run"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    assert!(project.calls().is_empty(), "{}", project.calls());
    assert!(
        !project.root().join("work").exists(),
        "a preview leaves no trace"
    );

    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--dry-run"]));
}

/// Выгрузка Конфигуратором по изменившемуся, перед которой поколение совпало с записанным
/// тем же инструментом, не запускает выгрузку: ответ «всё актуально».
#[test]
fn a_designer_pull_with_an_unchanged_generation_dumps_nothing() {
    let project = Project::new("File=ib");
    project.ignore_version_files();
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    assert!(project.sources.join("ConfigDumpInfo.xml").is_file());
    support::commit_sources(project.root());
    project.forget_calls();

    let pulled = succeeded(&project.run(&["pull", "main"]));

    assert_eq!(pulled["data"]["up_to_date"], true, "{pulled}");
    assert!(
        pulled["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nothing to dump"),
        "{pulled}"
    );
    assert!(
        !project.calls().contains("/DumpConfigToFiles"),
        "{}",
        project.calls()
    );

    project.base_generation(SECOND);
    let pulled = succeeded(&project.run(&["pull", "main"]));
    assert_eq!(pulled["data"]["up_to_date"], false, "{pulled}");
    assert!(project.calls().contains("/DumpConfigToFiles"));
}

/// Выгрузка `ibcmd` по изменившемуся пропускается так же: поколение до неё совпало с
/// записанным `ibcmd`.
#[test]
fn an_ibcmd_pull_with_an_unchanged_generation_dumps_nothing() {
    let project = Project::new("File=ib");
    let root = project.root().to_path_buf();
    write_shell_script(
        &root.join("ibcmd"),
        &format!(
            r#"printf '%s\n' "$*" >> '{calls}'
case "$*" in
  *'generation-id'*) cat '{token}'; exit 0 ;;
esac
exit 0"#,
            calls = root.join("calls.log").display(),
            token = root.join("token").display(),
        ),
    );
    fs::write(
        &project.config,
        format!(
            "workPath: work\nformat: DESIGNER\nproviders:\n  build: ibcmd\n  dump: ibcmd\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
            root.join("ibcmd").display()
        ),
    )
    .expect("config");
    fs::write(
        project.sources.join("ConfigDumpInfo.xml"),
        "<ConfigDumpInfo version=\"2.17\"/>\n",
    )
    .expect("version file");
    project.ignore_version_files();
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    assert_eq!(project.ledger()["main"]["tool"], "ibcmd");
    project.forget_calls();

    let pulled = succeeded(&project.run(&["pull", "main"]));

    assert_eq!(pulled["data"]["up_to_date"], true, "{pulled}");
    assert!(
        !project.calls().contains("dump"),
        "only the generation is asked: {}",
        project.calls()
    );
}

/// Загрузка упала, успев сдвинуть поколение: ответ говорит, что поколение не записано, а
/// следующая отправка отказывает `non_fast_forward` и называет неудачную загрузку, а не
/// «ушла вперёд с прошлого обмена». Не проходит она и без сверки.
#[test]
fn after_a_failed_load_the_next_push_names_it_and_is_not_let_through() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    fs::write(project.root().join("fail"), "").expect("fail");
    fs::write(project.root().join("drift"), format!("{SECOND}\n")).expect("drift");
    project.edit();

    let failed = envelope(&project.run(&["push"]));

    assert_eq!(failed["ok"], false, "{failed}");
    let message = failed["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("is not recorded"), "{message}");
    assert_eq!(project.ledger()["main"]["after"], "failed_build");
    assert_eq!(project.ledger()["main"]["token"], FIRST);
    fs::remove_file(project.root().join("fail")).expect("fail");
    fs::remove_file(project.root().join("drift")).expect("drift");
    project.forget_calls();

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("failed"), "{message}");
    assert!(!message.contains("since the last exchange"), "{message}");
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
}

/// Загрузка удалась, а поколения после неё нет: запись набора стирается, и ответ это
/// называет.
#[test]
fn without_an_answer_after_the_load_the_record_is_erased_and_named() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    fs::remove_file(project.root().join("token")).expect("token");
    project.edit();

    let pushed = succeeded(&project.run(&["push", "--force"]));

    let message = pushed["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("previous record is erased"), "{message}");
    assert!(
        project.ledger().get("main").is_none(),
        "{}",
        project.ledger()
    );
}

/// Выгрузка, о которой инструмент не ответил поколением, запись не меняет; выборка объектов
/// поколения не пишет вовсе.
#[test]
fn a_pull_without_an_answer_or_of_objects_leaves_the_record_as_it_was() {
    let project = Project::new("File=ib");
    project.ignore_version_files();
    support::commit_sources(project.root());
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    let recorded = project.ledger();
    support::commit_sources(project.root());

    project.base_generation(SECOND);
    succeeded(&project.run(&["pull", "main", "--object", "Catalog.Items"]));
    assert_eq!(project.ledger(), recorded);

    fs::remove_file(project.root().join("token")).expect("token");
    succeeded(&project.run(&["pull", "main", "--force"]));
    assert_eq!(project.ledger(), recorded);
}

/// База, созданная раннером, помнится: первая отправка в неё проходит без отказа первого
/// знакомства.
#[test]
fn a_base_created_by_the_runner_takes_the_first_push() {
    let project = Project::new("File=ib").with_extension();

    succeeded(&project.run(&["infobase", "create"]));
    let pushed = succeeded(&project.run(&["push"]));

    // Основную конфигурацию база получила при создании, и память её знает; расширение
    // досылает первая отправка целиком.
    let steps = pushed["data"]["steps"].as_array().expect("steps");
    let mode = |name: &str| {
        steps
            .iter()
            .find(|step| step["source_set"] == name)
            .map(|step| step["mode"].clone())
            .unwrap_or_else(|| panic!("{name} in {pushed}"))
    };
    assert_eq!(mode("main"), "skipped", "{pushed}");
    assert_eq!(mode("ext"), "full", "{pushed}");
}

/// Поколение всех наборов сверяется до первой загрузки: отказ по расширению приходит раньше,
/// чем основная конфигурация легла в базу.
#[test]
fn every_set_is_checked_before_the_first_load() {
    let project = Project::new("File=ib").with_extension();
    project.base_generation(FIRST);
    project.extension_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    assert_eq!(project.ledger()["ext"]["token"], FIRST);
    project.extension_generation(SECOND);
    project.edit();
    project.edit_extension();
    project.forget_calls();

    let refused = envelope(&project.run(&["push"]));

    assert_eq!(refused["error"]["code"], "non_fast_forward", "{refused}");
    assert_eq!(refused["error"]["next"]["source_set"], "ext", "{refused}");
    assert!(
        !project.calls().contains("/LoadConfigFromFiles"),
        "nothing is loaded before the refusal: {}",
        project.calls()
    );
    let steps = refused["data"]["steps"].as_array().expect("steps");
    assert_eq!(steps[0]["source_set"], "ext", "{refused}");
    assert!(
        !steps[0]["message"]
            .as_str()
            .unwrap_or_default()
            .starts_with("non-fast-forward"),
        "the step names the refusal once: {refused}"
    );
}

/// Своя запись поколения не спасает чужую хеш-память: каталог от этой базы она не выводит,
/// и `push` отказывает `no_memory`, а не идёт в анализ изменений.
#[test]
fn a_generation_record_does_not_make_memory_of_another_pair_own() {
    let project = Project::new("File=ib");
    project.base_generation(FIRST);
    succeeded(&project.run(&["push", "--force"]));
    let hashes = project
        .root()
        .join("work")
        .join("infobases")
        .join("origin")
        .join("hashes")
        .join("main.redb");
    let of_the_first_base = fs::read(&hashes).expect("hash memory");
    let local = project.root().join("v8project.local.yaml");
    fs::write(
        &local,
        fs::read_to_string(&local)
            .expect("local layer")
            .replace("File=ib", "File=replacement-ib"),
    )
    .expect("retarget");
    succeeded(&project.run(&["push", "--force"]));
    assert_eq!(project.ledger()["main"]["tool"], "designer");
    // Запись поколения — этой пары, хеш-память — прежней.
    fs::write(&hashes, of_the_first_base).expect("foreign hash memory");
    project.edit();
    project.forget_calls();

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("another infobase"), "{message}");
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// Конфигуратор у базы в кластере поколением заведомо не отвечает (#184): выгрузка поверх
/// каталога памяти не записала бы, и отказ без памяти советует полную `pull <SET> --force`,
/// предупреждая о потере незакоммиченного. У файловой базы совет — `pull <SET>`.
#[test]
fn without_a_generation_answer_a_no_memory_refusal_offers_pull_force() {
    let project = Project::new("Srvr=cluster;Ref=dev");

    let payload = envelope(&project.run(&["push"]));

    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    assert_eq!(payload["error"]["next"]["source_set"], "main", "{payload}");
    assert_eq!(payload["error"]["next"]["keys"]["--force"], "", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("pull main --force`"), "{message}");
    assert!(
        message.contains("discards its uncommitted changes"),
        "{message}"
    );

    let file = envelope(&Project::new("File=ib").run(&["push"]));
    assert_eq!(file["error"]["next"]["command"], "pull", "{file}");
    assert!(file["error"]["next"].get("keys").is_none(), "{file}");
}
