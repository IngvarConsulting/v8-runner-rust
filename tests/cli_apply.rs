//! `apply` отдельной командой и `push --no-apply` (#210).
//!
//! Поддельный Конфигуратор отвечает `/GetConfigGenerationID` токеном из файла `token` рядом с
//! собой. `/UpdateDBCfg` отказывает при файле `apply-fails`, а при файле `apply-token`
//! переносит его в `token` — так подделка меняет поколение при применении, как могла бы
//! платформа (замера, меняет ли, нет). Загрузка отказывает при файле `fail`, сдвинув
//! поколение на `drift`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";
const THIRD: &str = "3333333333333333333333333333333333333333";

fn platform(root: &Path) -> String {
    format!(
        r#"printf '%s\n' "$*" >> '{calls}'
out=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  previous="$arg"
done
case "$*" in
  *'/GetConfigGenerationID'*)
    if [ -f '{token}' ]; then cat '{token}' > "$out"; fi
    exit 0 ;;
  *'/UpdateDBCfg'*)
    if [ -f '{apply_fails}' ]; then exit 1; fi
    if [ -f '{apply_token}' ]; then mv '{apply_token}' '{token}'; fi ;;
  *'/LoadConfigFromFiles'*)
    if [ -f '{fail}' ]; then
      if [ -f '{drift}' ]; then cp '{drift}' '{token}'; fi
      exit 1
    fi ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
        token = root.join("token").display(),
        apply_fails = root.join("apply-fails").display(),
        apply_token = root.join("apply-token").display(),
        drift = root.join("drift").display(),
        fail = root.join("fail").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
        let ext = root.join("ext");
        fs::create_dir_all(&ext).expect("extension sources");
        fs::write(ext.join("Configuration.xml"), "<Configuration/>\n").expect("extension");
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
            "infobases:\n  origin:\n    connection: 'File=ib'\n",
        )
        .expect("local layer");
        let project = Self { dir, config };
        project.base_generation(FIRST);
        project
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn with_extension(self) -> Self {
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

    fn base_generation(&self, token: &str) {
        fs::write(self.root().join("token"), format!("{token}\r\n")).expect("token");
    }

    /// Поколение, которым база ответит после следующего применения.
    fn apply_moves_to(&self, token: &str) {
        fs::write(self.root().join("apply-token"), format!("{token}\r\n")).expect("token");
    }

    fn mark(&self, name: &str) {
        fs::write(self.root().join(name), "").expect("mark");
    }

    fn unmark(&self, name: &str) {
        fs::remove_file(self.root().join(name)).expect("unmark");
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
        let module = self.root().join("sources").join("Module.bsl");
        let text = fs::read_to_string(&module).expect("module");
        fs::write(&module, format!("{text}// edited\n")).expect("edit");
    }

    fn record(&self) -> Value {
        let file = self
            .root()
            .join("work")
            .join("infobases")
            .join("origin")
            .join("generation.json");
        let text = fs::read_to_string(file).expect("generation ledger");
        serde_json::from_str::<Value>(&text).expect("ledger json")["main"].clone()
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

/// `push --no-apply` грузит в основную конфигурацию и не применяет; запись поколения помнит
/// загрузку непринятой; `apply` применяет и снимает признак.
#[test]
fn a_push_without_apply_loads_and_apply_applies_it() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    project.forget_calls();

    let pushed = succeeded(&project.run(&["push", "--no-apply"]));

    let step = &pushed["data"]["steps"][0];
    assert_eq!(step["ok"], true, "{pushed}");
    assert_eq!(step["applied"], false, "{pushed}");
    assert!(
        step["message"]
            .as_str()
            .unwrap_or_default()
            .contains("loaded without apply"),
        "{pushed}"
    );
    assert!(project.calls().contains("/LoadConfigFromFiles"));
    assert!(
        !project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
    assert_eq!(project.record()["applied"], false);
    assert_eq!(project.record()["after"], "build");

    project.forget_calls();
    let applied = succeeded(&project.run(&["apply"]));

    assert_eq!(applied["command"], "apply", "{applied}");
    let step = &applied["data"]["steps"][0];
    assert_eq!(step["source_set"], "main", "{applied}");
    assert_eq!(step["outcome"], "applied", "{applied}");
    assert_eq!(step["generation"], "recorded", "{applied}");
    assert_eq!(applied["data"]["provider_dispatched"], true, "{applied}");
    assert!(
        project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
    assert!(
        project.record().get("applied").is_none(),
        "{}",
        project.record()
    );
}

/// Главная регрессия: применение, сменившее поколение, не делает следующую отправку чужой
/// правкой — `apply` переносит запись на ответ после себя.
#[test]
fn a_push_after_an_apply_that_changed_the_generation_is_not_refused() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    project.apply_moves_to(SECOND);

    succeeded(&project.run(&["apply"]));
    assert_eq!(project.record()["token"], SECOND);

    project.edit();
    let pushed = succeeded(&project.run(&["push"]));
    assert_eq!(pushed["data"]["steps"][0]["applied"], true, "{pushed}");
}

/// Выгрузка после `push --no-apply` видит то же поколение и ничего не берёт.
#[test]
fn a_pull_after_a_push_without_apply_is_up_to_date() {
    let project = Project::new();
    // Выгрузка по изменившемуся идёт только при файле версий: он принадлежит базе и в git
    // не хранится.
    fs::write(project.root().join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    fs::write(
        project.root().join("sources").join("ConfigDumpInfo.xml"),
        "<ConfigDumpInfo version=\"2.20\"/>\n",
    )
    .expect("version file");
    support::commit_sources(project.root());
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    support::commit_sources(project.root());
    project.forget_calls();

    let pulled = succeeded(&project.run(&["pull", "main"]));

    assert!(
        !project.calls().contains("/DumpConfigToFiles"),
        "{pulled}\n{}",
        project.calls()
    );
    assert_eq!(project.record()["applied"], false, "{}", project.record());
}

/// Отправка, которой нечего грузить, применяет своё непринятое, когда поколение базы равно
/// записи, — и не трогает базу, когда применять нечего.
#[test]
fn a_push_with_nothing_to_load_applies_its_own_unapplied() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    project.apply_moves_to(THIRD);
    project.forget_calls();

    let pushed = succeeded(&project.run(&["push"]));

    let step = &pushed["data"]["steps"][0];
    assert_eq!(step["mode"], "skipped", "{pushed}");
    assert_eq!(step["applied"], true, "{pushed}");
    assert!(
        project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
    assert_eq!(project.record()["token"], THIRD);
    assert!(project.record().get("applied").is_none());

    project.forget_calls();
    let again = succeeded(&project.run(&["push"]));
    assert_eq!(again["data"]["steps"][0]["applied"], false, "{again}");
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// Загрузка удалась, применение нет: память исходников и поколение запомнены непринятыми, а
/// отказ называет выход `apply`; после него обычная отправка проходит.
#[test]
fn a_push_whose_apply_failed_keeps_the_load() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    project.mark("apply-fails");

    let failed = project.run(&["push"]);
    let payload = envelope(&failed);

    assert!(!failed.status.success(), "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "apply", "{payload}");
    assert_eq!(payload["error"]["next"]["source_set"], "main", "{payload}");
    assert_eq!(payload["data"]["steps"][0]["applied"], false, "{payload}");
    assert_eq!(project.record()["applied"], false);
    assert_eq!(project.record()["after"], "build");

    project.unmark("apply-fails");
    succeeded(&project.run(&["apply", "main"]));
    project.forget_calls();
    let pushed = succeeded(&project.run(&["push"]));
    assert_eq!(pushed["data"]["steps"][0]["mode"], "skipped", "{pushed}");
    assert!(!project.calls().contains("/LoadConfigFromFiles"));
}

/// После неудачной загрузки, сдвинувшей поколение, `apply` отказывает `non_fast_forward` до
/// применения и первым выходом называет `push --force`.
#[test]
fn an_apply_after_a_failed_load_is_refused() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    project.mark("fail");
    fs::write(project.root().join("drift"), format!("{SECOND}\n")).expect("drift");
    assert!(!project.run(&["push"]).status.success());
    assert_eq!(project.record()["after"], "failed_build");
    project.forget_calls();

    let refused = project.run(&["apply"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "push", "{payload}");
    assert_eq!(payload["error"]["next"]["keys"]["--force"], "", "{payload}");
    assert_eq!(
        payload["data"]["steps"][0]["outcome"], "failed",
        "{payload}"
    );
    assert!(
        !project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
}

/// База ушла от записи до применения: `apply` применяет с предупреждением и запись не
/// трогает — следующая отправка откажет, пока базу не выгрузят или не перезапишут.
#[test]
fn an_apply_into_a_base_that_moved_keeps_the_record() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.base_generation(SECOND);

    let applied = succeeded(&project.run(&["apply"]));

    let step = &applied["data"]["steps"][0];
    assert_eq!(step["outcome"], "applied", "{applied}");
    assert_eq!(step["generation"], "kept", "{applied}");
    assert!(
        step["message"]
            .as_str()
            .unwrap_or_default()
            .contains("moved ahead of the record"),
        "{applied}"
    );
    assert_eq!(project.record()["token"], FIRST);
}

/// С набором `apply` применяет только его — у расширения с его именем.
#[test]
fn an_apply_of_one_set_applies_only_that_set() {
    let project = Project::new().with_extension();
    project.forget_calls();

    let applied = succeeded(&project.run(&["apply", "ext"]));

    let steps = applied["data"]["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 1, "{applied}");
    assert_eq!(steps[0]["source_set"], "ext", "{applied}");
    assert_eq!(steps[0]["purpose"], "EXTENSION", "{applied}");
    assert_eq!(steps[0]["generation"], "unchecked", "{applied}");
    let calls = project.calls();
    assert!(calls.contains("/UpdateDBCfg -Extension ext"), "{calls}");
    assert_eq!(calls.matches("/UpdateDBCfg").count(), 1, "{calls}");
}

/// Превью называет план и не запускает платформу.
#[test]
fn an_apply_preview_dispatches_nothing() {
    let project = Project::new();

    let planned = succeeded(&project.run(&["--dry-run", "apply"]));

    assert_eq!(planned["data"]["provider_dispatched"], false, "{planned}");
    assert_eq!(
        planned["data"]["steps"][0]["outcome"], "planned",
        "{planned}"
    );
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// `providers.apply` назначает исполнителя применения отдельно от `push`.
#[test]
fn providers_apply_assigns_the_executor_of_the_apply() {
    let project = Project::new();
    let ibcmd = project.root().join("ibcmd");
    write_shell_script(
        &ibcmd,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\nexit 0",
            project.root().join("ibcmd.log").display()
        ),
    );
    fs::write(
        &project.config,
        fs::read_to_string(&project.config)
            .expect("config")
            .replace("  apply: designer\n", "  apply: ibcmd\n"),
    )
    .expect("config");

    let applied = succeeded(&project.run(&["apply"]));

    assert_eq!(
        applied["data"]["provider"]["selected"], "ibcmd",
        "{applied}"
    );
    let calls = fs::read_to_string(project.root().join("ibcmd.log")).expect("ibcmd calls");
    assert!(calls.contains("config apply"), "{calls}");
    assert!(calls.contains("--force"), "{calls}");
    assert!(calls.contains("--dynamic auto"), "{calls}");
    assert!(
        !project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
}
