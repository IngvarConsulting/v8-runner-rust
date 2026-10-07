//! `status`, `status --all` и `status --deep` (#216).
//!
//! Поддельный Конфигуратор отвечает `/GetConfigGenerationID` токеном из файла `token` рядом
//! с собой; поддельный `ibcmd` отвечает `config extension list` текстом из файла `extensions`.
//! Оба пишут свои вызовы в общий журнал, по которому видно, запускалась ли платформа.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";

/// Состав расширений базы: `ext` проекта нет, есть `Проба`, которой нет в проекте.
const INSTALLED: &str = "name                         : \"Проба\"\nversion                      : \nactive                       : yes\npurpose                      : add-on\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"9hfFb6YVX2OwLKZaL1L69Eq0Vrg=\"\n\n";

fn designer(root: &Path) -> String {
    format!(
        r#"printf 'designer %s\n' "$*" >> '{calls}'
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
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
        token = root.join("token").display(),
    )
}

fn ibcmd(root: &Path) -> String {
    format!(
        r#"printf 'ibcmd %s\n' "$*" >> '{calls}'
case "$*" in
  *'extension list'*) cat '{extensions}'; exit 0 ;;
esac
exit 0"#,
        calls = root.join("calls.log").display(),
        extensions = root.join("extensions").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Project {
    /// Проект с набором `main`, расширением `ext` и файловой базой `origin`, а в местном слое
    /// — ещё база `test`.
    fn new() -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        for set in ["sources", "ext"] {
            let path = root.join(set);
            fs::create_dir_all(&path).expect("sources");
            fs::write(path.join("Configuration.xml"), "<Configuration/>\n").expect("source");
            fs::write(path.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
        }
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("bin");
        write_shell_script(&bin.join("1cv8"), &designer(root));
        write_shell_script(&bin.join("ibcmd"), &ibcmd(root));
        fs::write(root.join("extensions"), INSTALLED).expect("extensions");
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\n{designer_leads}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\n  - name: ext\n    type: EXTENSION\n    path: ext\ntools:\n  platform:\n    path: '{}'\n",
                bin.display(),
 designer_leads = support::DESIGNER_LEADS,
),
        )
        .expect("config");
        fs::write(
            root.join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n  test:\n    connection: 'File=ib-test'\n",
        )
        .expect("local layer");
        Self { dir, config }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

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

    /// Память о базе `origin`: отправка перезаписью с поколением `token`.
    fn remember(&self, token: &str) {
        self.base_generation(token);
        succeeded(&self.run(&["push", "--force"]));
        self.forget_calls();
    }

    /// Состав расширений базы по именам, в ответе `ibcmd config extension list`.
    fn installed(&self, names: &[&str]) {
        let text = names
            .iter()
            .map(|name| INSTALLED.replace("Проба", name))
            .collect::<String>();
        fs::write(self.root().join("extensions"), text).expect("extensions");
    }

    /// Ещё один набор расширения `name` в каталоге `name`.
    fn with_extension_set(self, name: &str) -> Self {
        let path = self.root().join(name);
        fs::create_dir_all(&path).expect("sources");
        fs::write(path.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        let text = fs::read_to_string(&self.config).expect("config").replace(
            "tools:\n",
            &format!("  - name: {name}\n    type: EXTENSION\n    path: {name}\ntools:\n"),
        );
        fs::write(&self.config, text).expect("config");
        self
    }

    /// Расширение-инструмент клиентского MCP `client_mcp`.
    fn with_tool_extension(self) -> Self {
        fs::write(self.root().join("client-mcp.cfe"), "cfe").expect("tool artifact");
        let mut text = fs::read_to_string(&self.config).expect("config");
        text.push_str("  client_mcp:\n    extension:\n      name: client_mcp\n      artifact:\n        path: client-mcp.cfe\n");
        fs::write(&self.config, text).expect("config");
        self
    }

    fn memory(&self) -> PathBuf {
        self.root().join("work").join("infobases").join("origin")
    }

    fn marker(&self) -> PathBuf {
        self.root().join(".ib.v8-runner.owners.json")
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

fn set<'a>(base: &'a Value, name: &str) -> &'a Value {
    base["source_sets"]
        .as_array()
        .expect("source sets")
        .iter()
        .find(|set| set["name"] == name)
        .unwrap_or_else(|| panic!("no source-set {name}: {base}"))
}

/// Читает файлы каталога целиком: что лежит под ним, побайтно.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap_or_default()));
            }
        }
    }
    files.sort();
    files
}

/// `status` отвечает по памяти: ни одна утилита платформы не запускается, а без платформы на
/// машине ответ тот же и отказом не становится.
#[test]
fn status_without_deep_starts_no_platform() {
    let project = Project::new();
    project.remember(FIRST);

    let status = succeeded(&project.run(&["status"]));
    assert!(
        project.calls().is_empty(),
        "status started the platform: {}",
        project.calls()
    );
    assert_data_matches_its_command_form(&status, "status");
    let base = &status["data"]["infobases"][0];
    assert_eq!(base["name"], "origin", "{status}");
    assert_eq!(set(base, "main")["memory"], "remembered", "{status}");
    assert_eq!(set(base, "main")["recorded"]["token"], FIRST, "{status}");
    assert_eq!(
        set(base, "main")["recorded"]["tool"],
        "designer",
        "{status}"
    );
    assert_eq!(set(base, "main")["changed_files"], 0, "{status}");
    assert!(base.get("extensions").is_none(), "{status}");

    fs::remove_dir_all(project.root().join("bin")).expect("no platform on the machine");
    fs::write(
        project.root().join("sources").join("Module.bsl"),
        "Procedure A()\n// edited\nEndProcedure\n",
    )
    .expect("edit");
    let without_platform = succeeded(&project.run(&["status"]));
    let base = &without_platform["data"]["infobases"][0];
    assert_eq!(
        set(base, "main")["memory"],
        "remembered",
        "{without_platform}"
    );
    assert_eq!(set(base, "main")["changed_files"], 1, "{without_platform}");

    let all = succeeded(&project.run(&["status", "--all"]));
    assert!(project.calls().is_empty(), "{}", project.calls());
    let bases = all["data"]["infobases"].as_array().expect("infobases");
    assert_eq!(bases.len(), 2, "{all}");
    assert_eq!(bases[0]["name"], "origin", "{all}");
    assert_eq!(bases[0]["selected"], true, "{all}");
    assert_eq!(bases[1]["name"], "test", "{all}");
    assert_eq!(bases[1]["selected"], false, "{all}");
    assert_eq!(set(&bases[1], "main")["memory"], "none", "{all}");
}

/// До первого обмена памяти о базе нет, и `status` так и говорит.
#[test]
fn status_names_a_base_without_memory() {
    let project = Project::new();

    let status = succeeded(&project.run(&["status"]));
    let base = &status["data"]["infobases"][0];
    assert_eq!(set(base, "main")["memory"], "none", "{status}");
    assert_eq!(set(base, "main")["recorded"], Value::Null, "{status}");
    assert_eq!(set(base, "main")["changed_files"], Value::Null, "{status}");
}

/// `status --deep` спрашивает поколение тем же инструментом, что `push`, и сверяет его с
/// записью так же: та же база — без изменений, ушедшая вперёд — `moved_ahead`, и тогда
/// `push` действительно отказывает `non_fast_forward`.
#[test]
fn status_deep_predicts_the_push_generation_check() {
    let project = Project::new();
    project.remember(FIRST);

    let unchanged = succeeded(&project.run(&["status", "--deep"]));
    assert_data_matches_its_command_form(&unchanged, "status --deep");
    let main = set(&unchanged["data"]["infobases"][0], "main");
    assert_eq!(main["base"]["tool"], "designer", "{unchanged}");
    assert_eq!(main["base"]["token"], FIRST, "{unchanged}");
    assert_eq!(main["base"]["comparison"], "unchanged", "{unchanged}");

    project.base_generation(SECOND);
    let moved = succeeded(&project.run(&["status", "--deep"]));
    let main = set(&moved["data"]["infobases"][0], "main");
    assert_eq!(main["base"]["token"], SECOND, "{moved}");
    assert_eq!(main["base"]["comparison"], "moved_ahead", "{moved}");

    fs::write(
        project.root().join("sources").join("Module.bsl"),
        "Procedure A()\n// edited\nEndProcedure\n",
    )
    .expect("edit");
    let refused = envelope(&project.run(&["push", "--source-set", "main"]));
    assert_eq!(refused["error"]["code"], "non_fast_forward", "{refused}");
}

/// `status --deep` называет расширение базы, которого нет в проекте, и набор проекта,
/// которого нет в базе.
#[test]
fn status_deep_names_an_extension_without_a_project() {
    let project = Project::new();
    project.remember(FIRST);

    let status = succeeded(&project.run(&["status", "--deep"]));
    let extensions = &status["data"]["infobases"][0]["extensions"];
    assert_eq!(extensions["provider"]["selected"], "ibcmd", "{status}");
    assert_eq!(extensions["installed"][0]["name"], "Проба", "{status}");
    assert_eq!(
        extensions["installed"][0]["source_set"],
        Value::Null,
        "{status}"
    );
    assert_eq!(extensions["missing_in_base"][0], "ext", "{status}");
}

/// `status --deep` называет копию, которая держит файловую базу, и ничего не пишет: ни в
/// метку, ни в память под `workPath`.
#[test]
fn status_deep_names_the_owning_copy_and_writes_nothing() {
    let project = Project::new();
    project.remember(FIRST);
    project.base_generation(SECOND);
    let marker = fs::read(project.marker()).expect("the push recorded the owner");
    let memory = snapshot(&project.memory());

    let status = succeeded(&project.run(&["status", "--deep"]));
    let holders = &status["data"]["infobases"][0]["holders"];
    assert_eq!(holders["owners"][0]["this_copy"], true, "{status}");
    assert_eq!(
        Path::new(holders["owners"][0]["project"].as_str().expect("project"))
            .canonicalize()
            .expect("project"),
        project.root().canonicalize().expect("root"),
        "{status}"
    );
    assert_eq!(fs::read(project.marker()).expect("marker"), marker);
    assert_eq!(snapshot(&project.memory()), memory);
}

/// На базе без метки `status --deep` владельцем не становится: метки после него нет.
#[test]
fn status_deep_on_a_base_without_a_marker_makes_no_owner() {
    let project = Project::new();
    fs::create_dir_all(project.root().join("ib")).expect("base");
    project.base_generation(FIRST);

    let status = succeeded(&project.run(&["status", "--deep"]));
    assert_eq!(
        status["data"]["infobases"][0]["holders"]["owners"],
        serde_json::json!([]),
        "{status}"
    );
    assert!(!project.marker().exists());
    assert!(project.calls().contains("/GetConfigGenerationID"));
}

/// `--deep` и `--all` не сочетаются: глубокий ответ — об одной базе.
#[test]
fn status_deep_and_all_do_not_combine() {
    let project = Project::new();
    let output = project.run(&["status", "--deep", "--all"]);
    assert!(!output.status.success());
    assert!(project.calls().is_empty());
}

/// Имена расширений сопоставляются без регистра — и латиница, и кириллица: набор `ext` —
/// то же расширение, что `EXT` в базе.
#[test]
fn status_deep_matches_extension_names_without_case() {
    let project = Project::new().with_extension_set("Расширение");
    project.remember(FIRST);
    project.installed(&["EXT", "РАСШИРЕНИЕ"]);

    let status = succeeded(&project.run(&["status", "--deep"]));
    let extensions = &status["data"]["infobases"][0]["extensions"];
    assert_eq!(extensions["installed"][0]["name"], "EXT", "{status}");
    assert_eq!(extensions["installed"][0]["source_set"], "ext", "{status}");
    assert_eq!(
        extensions["installed"][1]["source_set"], "Расширение",
        "{status}"
    );
    assert_eq!(
        extensions["missing_in_base"],
        serde_json::json!([]),
        "{status}"
    );
}

/// Расширение-инструмент клиентского MCP набором не объявляется, и `status --deep` не
/// выдаёт его за расширение без проекта: у него `tool: true`.
#[test]
fn status_deep_marks_the_client_mcp_tool_extension() {
    let project = Project::new().with_tool_extension();
    project.remember(FIRST);
    project.installed(&["ext", "Client_Mcp"]);

    let status = succeeded(&project.run(&["status", "--deep"]));
    let installed = &status["data"]["infobases"][0]["extensions"]["installed"];
    assert_eq!(installed[0]["tool"], false, "{status}");
    assert_eq!(installed[1]["name"], "Client_Mcp", "{status}");
    assert_eq!(installed[1]["source_set"], Value::Null, "{status}");
    assert_eq!(installed[1]["tool"], true, "{status}");
}

/// Без платформы `status --deep` не отказывает: чего платформа не ответила, форма называет
/// `null` с причиной.
#[test]
fn status_deep_without_a_platform_answers_null_with_a_reason() {
    let project = Project::new();
    project.remember(FIRST);
    fs::remove_dir_all(project.root().join("bin")).expect("no platform on the machine");

    let status = succeeded(&project.run(&["status", "--deep"]));
    assert_data_matches_its_command_form(&status, "status --deep without a platform");
    let base = &status["data"]["infobases"][0];
    let main = set(base, "main");
    assert_eq!(main["base"]["token"], Value::Null, "{status}");
    assert_eq!(main["base"]["comparison"], "no_answer", "{status}");
    assert!(main["base"]["reason"].is_string(), "{status}");
    assert_eq!(base["extensions"]["installed"], Value::Null, "{status}");
    assert!(base["extensions"]["reason"].is_string(), "{status}");
}

/// Где `push` этим исполнителем проект не грузит (EDT и агент), сверки нет: `status --deep`
/// не спрашивает поколение и отвечает `null` с причиной — тем же отказом, что дал бы `push`.
#[test]
fn status_deep_where_push_does_not_load_with_this_executor_answers_null_with_a_reason() {
    let project = Project::new();
    let config = fs::read_to_string(&project.config).expect("config");
    fs::write(
        &project.config,
        config.replace(
            "format: DESIGNER\nproviders:\n  push: designer\n",
            "format: EDT\nproviders:\n  push: agent\n",
        ),
    )
    .expect("EDT project pushed by the agent");
    for (set, nature) in [
        ("sources", "V8ConfigurationNature"),
        ("ext", "V8ExtensionNature"),
    ] {
        fs::write(
            project.root().join(set).join(".project"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{set}</name>\n  <natures>\n    <nature>com._1c.g5.v8.dt.core.{nature}</nature>\n  </natures>\n</projectDescription>\n"
            ),
        )
        .expect("EDT project file");
        let dir = project.root().join(set);
        fs::create_dir_all(dir.join("DT-INF")).expect("DT-INF");
        fs::write(
            dir.join("DT-INF/PROJECT.PMF"),
            "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
        )
        .expect("manifest");
        fs::create_dir_all(dir.join("src/Configuration")).expect("src");
        fs::write(
            dir.join("src/Configuration/Configuration.mdo"),
            "<Configuration />\n",
        )
        .expect("root object");
    }

    let status = succeeded(&project.run(&["status", "--deep"]));
    assert_data_matches_its_command_form(&status, "status --deep, EDT pushed by the agent");
    let main = set(&status["data"]["infobases"][0], "main");
    assert_eq!(main["base"]["token"], Value::Null, "{status}");
    assert_eq!(main["base"]["tool"], "agent", "{status}");
    // Записи у EDT-набора нет, поэтому сверка без ответа называется `no_record`.
    assert_eq!(main["base"]["comparison"], "no_record", "{status}");
    let reason = main["base"]["reason"].as_str().expect("a reason");
    assert!(reason.contains("format=EDT"), "{status}");
    assert!(
        !project.calls().contains("GetConfigGenerationID"),
        "no generation is asked: {}",
        project.calls()
    );
}
