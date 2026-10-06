mod support;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command};

const V8_EXTERNAL_OBJECTS_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExternalObjectsNature";
const LOCAL_CONFIG_SCHEMA_MODEL_LINE: &str = "# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json";

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create dst");
    for entry in fs::read_dir(src).expect("read dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            copy_dir_all(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy file");
        }
    }
}

fn edt_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("edt")
}

fn copy_native_edt_fixture(dest_root: &Path) {
    let fixture_root = edt_fixture_root();
    copy_dir_all(
        &fixture_root.join("configuration"),
        &dest_root.join("configuration"),
    );
    copy_dir_all(
        &fixture_root.join("extension"),
        &dest_root.join("extension"),
    );
}

fn create_native_edt_external_project(project_dir: &Path, name: &str, descriptor_xml: &str) {
    fs::create_dir_all(project_dir.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::write(
        project_dir.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>{V8_EXTERNAL_OBJECTS_NATURE}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        project_dir.join("DT-INF").join("PROJECT.PMF"),
        "Base-Project: configuration\nManifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("manifest");
    fs::write(project_dir.join("src").join("root.xml"), descriptor_xml).expect("descriptor");
}

#[test]
fn config_init_creates_yaml_with_detected_designer_sources() {
    let dir = temp_workspace();
    let main = dir.path().join("src").join("configuration");
    let ext = dir.path().join("extensions").join("sales");
    fs::create_dir_all(&main).expect("main");
    fs::create_dir_all(&ext).expect("ext");
    fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");
    fs::write(
        ext.join("Configuration.xml"),
        "<Configuration><Properties><Name>SalesAddon</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
    )
    .expect("ext xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.starts_with(
        "# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.schema.json\n"
    ));
    serde_yaml::from_str::<serde_yaml::Value>(&config).expect("generated config remains YAML");
    assert!(config.contains("format: DESIGNER"));
    assert!(!config.contains("basePath:"));
    assert!(config.contains("workPath: 'build'"));
    assert!(
        !config.contains("infobase"),
        "the project file names no base:\n{config}"
    );
    assert!(config.contains("#     wait_ready_timeout_ms: 300000"));
    assert!(config.contains("path: 'src/configuration'"));
    assert!(config.contains("name: 'SalesAddon'"));
    assert!(config.contains("type: EXTENSION"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Config written"));
    let local_config =
        fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
    assert!(local_config.starts_with(LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    assert!(
        local_config.contains("infobases:\n  origin:\n    connection: 'File=build/ib'\n"),
        "the local layer declares origin:\n{local_config}"
    );
    serde_yaml::from_str::<serde_yaml::Value>(&local_config)
        .expect("generated local config remains YAML");
    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
    assert!(gitignore.lines().any(|line| line == "v8project.local.yaml"));
}

#[test]
fn config_init_uses_json_envelope_and_output_override() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");
    let config_path = dir.path().join("custom.yaml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args([
            "--json-message",
            "config",
            "init",
            "--output",
            &config_path.display().to_string(),
            "--connection",
            &format!("File={tmp}/test-ib"),
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(config_path.exists());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "init");
    // Живая сверка формы: схема держит состав `data` только вместе с прогоном, иначе
    // команда вправе печатать не то, что за ней объявлено.
    assert_data_matches_its_command_form(&payload, "`config init --output`");
    let canonical_dir = fs::canonicalize(dir.path()).expect("canonical project dir");
    assert_eq!(
        payload["data"]["local_path"],
        canonical_dir
            .join("v8project.local.yaml")
            .display()
            .to_string()
    );
    assert_eq!(
        payload["data"]["gitignore_path"],
        canonical_dir.join(".gitignore").display().to_string()
    );
    assert_eq!(payload["data"]["source_sets"][0]["path"], ".");
    assert_eq!(payload["data"]["source_sets"][0]["type"], "CONFIGURATION");
    let config = fs::read_to_string(config_path).expect("config");
    assert!(!config.contains("infobase"), "{config}");
    assert!(!config.contains("basePath:"));
    let local_config = fs::read_to_string(
        payload["data"]["local_path"]
            .as_str()
            .expect("local path in the payload"),
    )
    .expect("local config");
    assert!(
        local_config.contains(&format!(
            "infobases:\n  origin:\n    connection: 'File={tmp}/test-ib'\n"
        )),
        "{local_config}"
    );
}

#[test]
fn config_init_creates_local_overlay_next_to_output_override() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init", "--output", "config/v8project.yaml"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config =
        fs::read_to_string(dir.path().join("config").join("v8project.yaml")).expect("config");
    assert!(!config.contains("basePath:"));
    assert!(config.contains("path: '..'"));
    let local_config = fs::read_to_string(dir.path().join("config").join("v8project.local.yaml"))
        .expect("local config");
    assert!(local_config.starts_with(LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    // Вне гита шаблоны тоже ложатся в `.gitignore` каталога проекта: шаблон без `/`
    // оттуда покрывает и вложенный конфиг, и каталоги наборов.
    assert!(!dir.path().join("config").join(".gitignore").exists());
    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
    assert!(gitignore.lines().any(|line| line == "v8project.local.yaml"));
}

#[test]
fn config_init_rejects_global_config_shortcut_in_text_mode() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--config", "custom.yaml", "config", "init"])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("global --config flag is not supported for `init`; use `init --output <FILE>`"));
}

#[test]
fn config_init_rejects_global_config_shortcut_in_json_mode() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args([
            "--config",
            "custom.yaml",
            "--json-message",
            "config",
            "init",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "init");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("use `init --output <FILE>`"));
}

#[test]
fn config_init_ignores_v8tr_config_env_for_output_path_selection() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .env("V8TR_CONFIG", dir.path().join("existing.yaml"))
        .args(["config", "init"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(dir.path().join("v8project.yaml").exists());
    assert!(!dir.path().join("existing.yaml").exists());
}

#[test]
fn config_init_detects_native_edt_fixture_source_sets() {
    let dir = temp_workspace();
    let workspace = dir.path().join("workspace");
    copy_native_edt_fixture(&workspace);

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("format: EDT"));
    assert!(config.contains("tools:\n  platform:\n    version: '8.3.27'"));
    assert!(config.contains("path: 'workspace/configuration'"));
    assert!(config.contains("path: 'workspace/extension'"));
    assert!(config.contains("name: 'Расширение1'"));
    assert!(config.contains("type: CONFIGURATION"));
    assert!(config.contains("type: EXTENSION"));
}

#[test]
fn config_init_detects_edt_extension_without_base_project_and_warns() {
    let dir = temp_workspace();
    let workspace = dir.path().join("workspace");
    copy_native_edt_fixture(&workspace);
    fs::write(
        workspace
            .join("extension")
            .join("DT-INF")
            .join("PROJECT.PMF"),
        "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("manifest");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--no-color", "config", "init"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("source-set Расширение1: workspace/extension (EXTENSION)"));
    assert!(stdout.contains("platform version: 8.3.27"));
    assert!(stdout.contains("[warning] EDT extension source-set 'Расширение1'"));
    assert!(stdout.contains("Base-Project"));
    assert!(stdout.contains("Config written with warnings"));

    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("tools:\n  platform:\n    version: '8.3.27'"));
    assert!(config.contains("name: 'Расширение1'"));
    assert!(config.contains("path: 'workspace/extension'"));
    assert!(config.contains("type: EXTENSION"));

    let json_output = v8_runner_command()
        .current_dir(dir.path())
        .args([
            "--json-message",
            "config",
            "init",
            "--force",
            "--output",
            "json-v8project.yaml",
        ])
        .output()
        .expect("run json command");

    assert!(json_output.status.success());
    let payload: Value = serde_json::from_slice(&json_output.stdout).expect("json");
    assert_eq!(payload["data"]["platform_version"], "8.3.27");
    let source_sets = payload["data"]["source_sets"]
        .as_array()
        .expect("source sets");
    assert!(source_sets.iter().any(|source_set| {
        source_set["name"] == "Расширение1"
            && source_set["path"] == "workspace/extension"
            && source_set["type"] == "EXTENSION"
    }));
    assert!(payload["data"]["warnings"][0]
        .as_str()
        .expect("warning")
        .contains("Base-Project"));
    assert!(payload["warnings"][0]
        .as_str()
        .expect("envelope warning")
        .contains("Base-Project"));
}

/// В объявленном проекте `init` проектный файл не трогает и пишет местный слой; ответ —
/// вариант `local`, без полей проектного файла.
#[test]
fn init_in_a_declared_project_leaves_the_project_file_and_writes_the_local_layer() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");
    fs::write(dir.path().join("v8project.yaml"), "existing").expect("existing");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init"])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("origin: declared (File=build/ib)"),
        "{stdout}"
    );
    assert!(!stdout.contains("Config written"), "{stdout}");
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        "existing"
    );
    let local = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local");
    assert!(
        local.contains("infobases:\n  origin:\n    connection: 'File=build/ib'\n"),
        "{local}"
    );

    let json_output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "config", "init"])
        .output()
        .expect("run json command");

    assert!(json_output.status.success());
    let payload: Value = serde_json::from_slice(&json_output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "init");
    assert_data_matches_its_command_form(&payload, "`init` in a declared project");
    let data = &payload["data"];
    assert_eq!(data["kind"], "local", "{payload}");
    for project_field in ["path", "format", "source_sets", "overwritten"] {
        assert!(
            data.get(project_field).is_none(),
            "`{project_field}` describes only a written project file: {payload}"
        );
    }
    assert_eq!(data["origin"]["change"], "unchanged", "{payload}");
    assert_eq!(data["origin"]["connection"], "File=build/ib", "{payload}");
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        "existing"
    );
}

/// Сценарий «Задача в новой рабочей копии» до `infobase create`: проектный файл пришёл с
/// веткой, местный слой скопирован у соседа. `init --infobase` отдаёт `origin` свой адрес,
/// прежнюю секцию с учётными данными сохраняет как `upstream`, называет заменённый адрес
/// и не печатает учётных данных ни текстом, ни в JSON.
#[test]
fn init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream() {
    let neighbour = temp_workspace();
    fs::write(
        neighbour.path().join("Configuration.xml"),
        "<Configuration/>",
    )
    .expect("xml");
    let declared = v8_runner_command()
        .current_dir(neighbour.path())
        .args(["init"])
        .output()
        .expect("declare the neighbour project");
    assert!(declared.status.success());
    let project_file =
        fs::read(neighbour.path().join("v8project.yaml")).expect("neighbour project file");
    // Пароль с пробелом — в кавычках и без них: хвост после пробела тоже пароль.
    let copied_origins = [
        (
            r#"Srvr=srv;Ref=erp;Usr=copy-user;Pwd="pwhead quotedtail""#,
            "Srvr=srv;Ref=erp;Usr=***;Pwd=***",
        ),
        (
            "Srvr=srv;Pwd=pwhead baretail;Usr=copy-user;Ref=erp",
            "Srvr=srv;Pwd=***;Usr=***;Ref=erp",
        ),
    ];

    for (copied_connection, shown_replaced) in copied_origins {
        for json in [false, true] {
            let worktree = temp_workspace();
            fs::write(
                worktree.path().join("Configuration.xml"),
                "<Configuration/>",
            )
            .expect("xml");
            fs::write(worktree.path().join("v8project.yaml"), &project_file).expect("project file");
            let copied_layer = format!(
                "infobases:\n  origin:\n    connection: '{copied_connection}'\n    user: copy-user\n    password: layer-secret\n"
            );
            fs::write(worktree.path().join("v8project.local.yaml"), &copied_layer).expect("layer");

            let mut command = v8_runner_command();
            command.current_dir(worktree.path());
            if json {
                command.arg("--json-message");
            }
            let output = command
                .args(["init", "--infobase", "File=build/ib"])
                .output()
                .expect("run init");

            let printed = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.status.success(), "{printed}");
            for secret in [
                "pwhead",
                "quotedtail",
                "baretail",
                "layer-secret",
                "copy-user",
            ] {
                assert!(
                    !printed.contains(secret),
                    "credentials are printed ({secret}): {printed}"
                );
            }
            if json {
                let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
                assert_data_matches_its_command_form(&payload, "`init --infobase` in a worktree");
                let data = &payload["data"];
                assert_eq!(data["kind"], "local", "{payload}");
                assert_eq!(data["origin"]["change"], "redirected", "{payload}");
                assert_eq!(data["origin"]["connection"], "File=build/ib", "{payload}");
                assert_eq!(data["origin"]["replaced"], shown_replaced, "{payload}");
            } else {
                assert!(
                    printed.contains("origin: redirected (File=build/ib)"),
                    "{printed}"
                );
                assert!(
                    printed.contains(&format!("upstream: {shown_replaced}")),
                    "the replaced address: {printed}"
                );
            }
            assert_eq!(
                fs::read(worktree.path().join("v8project.yaml")).expect("project file"),
                project_file,
                "the project file is left byte for byte"
            );
            let layer =
                fs::read_to_string(worktree.path().join("v8project.local.yaml")).expect("layer");
            let document: serde_yaml::Value = serde_yaml::from_str(&layer).expect("layer is YAML");
            assert_eq!(
                document["infobases"]["origin"]["connection"].as_str(),
                Some("File=build/ib"),
                "{layer}"
            );
            let upstream = &document["infobases"]["upstream"];
            assert_eq!(
                upstream["connection"].as_str(),
                Some(copied_connection),
                "{layer}"
            );
            assert_eq!(upstream["user"].as_str(), Some("copy-user"), "{layer}");
            assert_eq!(
                upstream["password"].as_str(),
                Some("layer-secret"),
                "{layer}"
            );
        }
    }
}

/// Существующий `upstream` не перезаписывается: отказ называет `origin` и `upstream` и
/// оставляет оба файла как были.
#[test]
fn init_refuses_to_redirect_origin_over_an_existing_upstream() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");
    fs::write(dir.path().join("v8project.yaml"), "existing").expect("existing");
    let layer = "infobases:\n  origin:\n    connection: 'File=/srv/ib'\n  upstream:\n    connection: 'File=/srv/older-ib'\n    password: layer-secret\n";
    fs::write(dir.path().join("v8project.local.yaml"), layer).expect("layer");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobases.origin"), "{message}");
    assert!(message.contains("infobases.upstream"), "{message}");
    assert!(!message.contains("layer-secret"), "{message}");
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("layer"),
        layer
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        "existing"
    );
}

/// Адрес, который уже стоит в `origin`, ничего не меняет и в объявленном проекте: ни
/// содержимого слоя, ни самого файла. Пробелы вокруг названного адреса не делают его другим.
#[test]
fn init_with_the_address_already_in_origin_changes_nothing() {
    let dir = temp_workspace();
    fs::write(dir.path().join("v8project.yaml"), "existing").expect("existing");
    let layer = format!(
        "{LOCAL_CONFIG_SCHEMA_MODEL_LINE}\ninfobases:\n  origin:\n    connection: 'File=build/ib'\n"
    );
    let layer_path = dir.path().join("v8project.local.yaml");
    fs::write(&layer_path, &layer).expect("layer");
    let long_ago =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(&layer_path)
        .expect("open layer")
        .set_modified(long_ago)
        .expect("age the layer");

    for address in ["File=build/ib", "  File=build/ib "] {
        let output = v8_runner_command()
            .current_dir(dir.path())
            .args(["--json-message", "init", "--infobase", address])
            .output()
            .expect("run init");

        assert!(output.status.success(), "{address:?}");
        let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(
            payload["data"]["origin"]["change"], "unchanged",
            "{address:?}: {payload}"
        );
        assert!(
            payload["data"]["origin"].get("replaced").is_none(),
            "{payload}"
        );
        assert_eq!(fs::read_to_string(&layer_path).expect("layer"), layer);
        assert_eq!(
            fs::metadata(&layer_path)
                .expect("layer metadata")
                .modified()
                .expect("mtime"),
            long_ago,
            "{address:?}: the layer is not rewritten"
        );
    }
    assert!(!dir.path().join("build").exists());
}

#[test]
fn config_init_detects_designer_external_aggregate_source_set() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("config xml");
    fs::create_dir_all(dir.path().join("tools")).expect("tools dir");
    fs::write(
        dir.path().join("tools").join("alpha.xml"),
        "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
    )
    .expect("alpha xml");
    fs::write(
        dir.path().join("tools").join("beta.xml"),
        "<MetaDataObject><ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor></MetaDataObject>",
    )
    .expect("beta xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init", "--format", "designer"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("type: EXTERNAL_DATA_PROCESSORS"));
    assert!(config.contains("path: 'tools'"));
}

#[test]
fn config_init_rejects_external_only_autodiscovery_without_configuration() {
    let dir = temp_workspace();
    fs::create_dir_all(dir.path().join("tools")).expect("tools dir");
    fs::write(
        dir.path().join("tools").join("alpha.xml"),
        "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
    )
    .expect("alpha xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init", "--format", "designer"])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("did not find a CONFIGURATION source-set")
    );
}

#[test]
fn config_init_auto_prefers_edt_when_designer_only_has_external_root() {
    let dir = temp_workspace();
    let workspace = dir.path().join("workspace");
    copy_dir_all(
        &edt_fixture_root().join("configuration"),
        &workspace.join("configuration"),
    );
    fs::create_dir_all(dir.path().join("tools")).expect("tools dir");
    fs::write(
        dir.path().join("tools").join("alpha.xml"),
        "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
    )
    .expect("alpha xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("format: EDT"));
    assert!(config.contains("path: 'workspace/configuration'"));
    assert!(!config.contains("path: 'tools'"));
}

#[test]
fn config_init_keeps_nested_edt_configuration_under_external_root() {
    let dir = temp_workspace();
    let external_root = dir.path().join("processors");
    for name in ["alpha", "beta"] {
        let project = external_root.join(name);
        create_native_edt_external_project(
            &project,
            name,
            &format!(
                "<ExternalDataProcessor><Properties><Name>{name}</Name></Properties></ExternalDataProcessor>"
            ),
        )
    }
    let config_project = external_root.join("apps").join("cfg");
    copy_dir_all(&edt_fixture_root().join("configuration"), &config_project);

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init", "--format", "edt"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("path: 'processors'"));
    assert!(config.contains("type: EXTERNAL_DATA_PROCESSORS"));
    assert!(config.contains("path: 'processors/apps/cfg'"));
    assert!(config.contains("type: CONFIGURATION"));
}

#[test]
fn config_init_ignores_non_edt_root_project_marker_when_nested_project_exists() {
    let dir = temp_workspace();
    fs::write(dir.path().join(".project"), "<root/>").expect("root project marker");
    let workspace = dir.path().join("workspace");
    copy_dir_all(
        &edt_fixture_root().join("configuration"),
        &workspace.join("configuration"),
    );

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["config", "init", "--format", "edt"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(dir.path().join("v8project.yaml")).expect("config");
    assert!(config.contains("path: 'workspace/configuration'"));
    assert!(config.contains("type: CONFIGURATION"));
}

/// Проектный файл со старым ключом `infobase:` и местный слой с полями той же секции:
/// `infobase:` — синоним `infobases.origin`, слои сливаются по полям, как у загрузчика.
const SYNONYM_PROJECT_FILE: &str = "workPath: build\nformat: DESIGNER\ninfobase:\n  connection: 'Srvr=srv;Ref=erp;Pwd=conn-secret'\n  user: proj-user\n  password: proj-secret\nsource-set: []\n";
const SYNONYM_LOCAL_LAYER: &str = "infobases:\n  origin:\n    password: layer-secret\n";
const SYNONYM_SECRETS: [&str; 4] = ["conn-secret", "proj-user", "proj-secret", "layer-secret"];

fn synonym_project() -> tempfile::TempDir {
    let dir = temp_workspace();
    fs::write(dir.path().join("v8project.yaml"), SYNONYM_PROJECT_FILE).expect("project file");
    fs::write(dir.path().join("v8project.local.yaml"), SYNONYM_LOCAL_LAYER).expect("layer");
    dir
}

fn printed(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Старый ключ `infobase:` в проектном файле объявляет `origin`: `init` без адреса ничего
/// не меняет ни в проекте, ни в местном слое, отвечает `unchanged` с действующим адресом и
/// предупреждает о синониме.
#[test]
fn init_over_the_infobase_synonym_in_the_project_file_keeps_the_declared_origin() {
    let dir = synonym_project();

    for json in [true, false] {
        let mut command = v8_runner_command();
        command.current_dir(dir.path());
        if json {
            command.arg("--json-message");
        }
        let output = command.arg("init").output().expect("run init");

        let printed = printed(&output);
        assert!(output.status.success(), "{printed}");
        for secret in SYNONYM_SECRETS {
            assert!(!printed.contains(secret), "{secret} is printed: {printed}");
        }
        if json {
            let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
            assert_data_matches_its_command_form(&payload, "`init` over the `infobase:` synonym");
            assert_eq!(payload["data"]["kind"], "local", "{payload}");
            assert_eq!(
                payload["data"]["origin"]["change"], "unchanged",
                "{payload}"
            );
            assert_eq!(
                payload["data"]["origin"]["connection"], "Srvr=srv;Ref=erp;Pwd=***",
                "{payload}"
            );
            assert!(
                payload["warnings"]
                    .as_array()
                    .expect("warnings")
                    .iter()
                    .any(|warning| warning.as_str().is_some_and(|w| w.contains("`infobase:`"))),
                "the synonym warning stays: {payload}"
            );
        } else {
            assert!(
                printed.contains("origin: unchanged (Srvr=srv;Ref=erp;Pwd=***)"),
                "{printed}"
            );
            assert!(printed.contains("[warning] `infobase:`"), "{printed}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
            SYNONYM_PROJECT_FILE
        );
        let layer = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("layer");
        assert_eq!(
            layer
                .strip_prefix(LOCAL_CONFIG_SCHEMA_MODEL_LINE)
                .and_then(|layer| layer.strip_prefix('\n')),
            Some(SYNONYM_LOCAL_LAYER),
            "the layer only gains its schema line: {layer}"
        );
    }
}

/// `init --infobase` в проекте со старым ключом: `origin` местного слоя получает новый
/// адрес, а действующая прежняя секция — слитая из обоих слоёв, с учётными данными —
/// уходит в `upstream`. Проектный файл не меняется, учётные данные не печатаются.
#[test]
fn init_with_an_infobase_redirects_the_origin_declared_by_the_infobase_synonym() {
    let dir = synonym_project();

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");

    let printed = printed(&output);
    assert!(output.status.success(), "{printed}");
    for secret in SYNONYM_SECRETS {
        assert!(!printed.contains(secret), "{secret} is printed: {printed}");
    }
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_data_matches_its_command_form(&payload, "`init --infobase` over the synonym");
    let origin = &payload["data"]["origin"];
    assert_eq!(origin["change"], "redirected", "{payload}");
    assert_eq!(origin["connection"], "File=build/ib", "{payload}");
    assert_eq!(origin["replaced"], "Srvr=srv;Ref=erp;Pwd=***", "{payload}");
    assert!(
        payload["warnings"][0]
            .as_str()
            .is_some_and(|warning| warning.contains("`infobase:`")),
        "{payload}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        SYNONYM_PROJECT_FILE
    );
    let layer = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("layer");
    let document: serde_yaml::Value = serde_yaml::from_str(&layer).expect("layer is YAML");
    assert_eq!(
        document["infobases"]["origin"]["connection"].as_str(),
        Some("File=build/ib"),
        "{layer}"
    );
    let upstream = &document["infobases"]["upstream"];
    assert_eq!(
        upstream["connection"].as_str(),
        Some("Srvr=srv;Ref=erp;Pwd=conn-secret"),
        "{layer}"
    );
    assert_eq!(upstream["user"].as_str(), Some("proj-user"), "{layer}");
    assert_eq!(
        upstream["password"].as_str(),
        Some("layer-secret"),
        "the local field wins, as in the loader: {layer}"
    );
}

/// После `init --infobase` загрузчик по-прежнему подмешивает в новый `origin` поля
/// проектной секции `infobase:`, кроме адреса. Ответ предупреждает об этом и называет
/// только имена полей; секция из одного адреса предупреждения не даёт.
#[test]
fn init_with_an_infobase_warns_which_project_fields_still_apply_to_the_new_origin() {
    let dir = synonym_project();
    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");
    let text = printed(&output);
    assert!(output.status.success(), "{text}");
    let warning = text
        .lines()
        .find(|line| line.contains("[warning]") && line.contains("redirected infobases.origin"))
        .unwrap_or_else(|| panic!("the inherited fields are named: {text}"));
    assert!(warning.contains("`user`"), "{warning}");
    assert!(warning.contains("`password`"), "{warning}");
    assert!(!warning.contains("`connection`"), "{warning}");
    assert!(warning.contains("v8project.local.yaml"), "{warning}");
    for secret in SYNONYM_SECRETS {
        assert!(!text.contains(secret), "{secret} is printed: {text}");
    }

    let dir = temp_workspace();
    fs::write(
        dir.path().join("v8project.yaml"),
        "workPath: build\ninfobase:\n  connection: 'File=/srv/ib'\n",
    )
    .expect("project file");
    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");
    assert!(output.status.success(), "{}", printed(&output));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_eq!(
        payload["data"]["origin"]["change"], "redirected",
        "{payload}"
    );
    let warnings = payload["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().all(|warning| !warning
            .as_str()
            .is_some_and(|warning| warning.contains("redirected infobases.origin"))),
        "an address alone leaves nothing to inherit: {payload}"
    );
}

/// Занятый `upstream` не перезаписывается и тогда, когда `origin` объявлен старым ключом
/// проектного файла: отказ, оба файла как были.
#[test]
fn init_refuses_to_redirect_the_infobase_synonym_over_an_existing_upstream() {
    let dir = temp_workspace();
    fs::write(dir.path().join("v8project.yaml"), SYNONYM_PROJECT_FILE).expect("project file");
    let layer = "infobases:\n  upstream:\n    connection: 'File=/srv/older-ib'\n    password: layer-secret\n";
    fs::write(dir.path().join("v8project.local.yaml"), layer).expect("layer");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2), "{}", printed(&output));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobases.origin"), "{message}");
    assert!(message.contains("infobases.upstream"), "{message}");
    for secret in SYNONYM_SECRETS {
        assert!(!message.contains(secret), "{message}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("layer"),
        layer
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        SYNONYM_PROJECT_FILE
    );
}

/// Карта `infobases` в проектном файле — отказ загрузчика, и `init` отказывает тем же
/// отказом, ничего не записав.
#[test]
fn init_refuses_the_infobases_map_in_the_project_file_as_the_loader_does() {
    let dir = temp_workspace();
    let project = "workPath: build\ninfobases:\n  origin:\n    connection: 'File=build/ib'\n";
    fs::write(dir.path().join("v8project.yaml"), project).expect("project file");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "init"])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2), "{}", printed(&output));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("v8project.local.yaml"), "{message}");
    assert_eq!(payload["command"], "init", "{payload}");
    assert!(!dir.path().join("v8project.local.yaml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.yaml")).expect("project file"),
        project
    );
}

/// Порождённый конфиг не пишет ни прежнего имени секции — иначе первая же следующая
/// команда предупреждала бы о синониме, который выписал сам раннер, — ни ключа порога
/// частичной загрузки, который валидация отвергает
/// (`INV.CONFIG.PARTIAL-LOAD-THRESHOLD-KEY-IS-REJECTED`).
#[test]
fn a_generated_config_writes_neither_a_synonym_nor_a_retired_key() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["init"])
        .output()
        .expect("run init");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(dir.path().join("v8project.yaml")).expect("generated");
    assert!(!generated.contains("build:"), "{generated}");
    assert!(!generated.contains("partialLoadThreshold"), "{generated}");
    assert!(
        generated.contains("# Generated by v8-runner init\n"),
        "the header names the command that wrote the file:\n{generated}"
    );
}

/// `init` объявляет базу, а не выбирает её: глобальный ключ здесь называет адрес, который
/// уезжает в `infobases.origin` местного слоя.
#[test]
fn init_writes_the_address_named_by_the_global_key_into_origin() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["init", "--infobase", "Srvr=srv;Ref=erp"])
        .output()
        .expect("run init");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let local = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local");
    // Адрес проверяется по пути, а не по строке: `connection` в другом месте документа
    // подстроку даст, а базу не объявит.
    let document: serde_yaml::Value = serde_yaml::from_str(&local).expect("local is YAML");
    assert_eq!(
        document["infobases"]["origin"]["connection"].as_str(),
        Some("Srvr=srv;Ref=erp"),
        "{local}"
    );
}

/// Имя базы разрешать не по чему: местного слоя ещё нет, и `init` отвечает отказом.
#[test]
fn init_refuses_a_base_named_by_name_because_it_has_nothing_to_resolve_it_against() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["init", "--infobase", "test"])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2));
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(reported.contains("connection string"), "{reported}");
    assert!(reported.contains("test"), "{reported}");
    assert!(
        !dir.path().join("v8project.yaml").exists(),
        "отказ случается до того, как проект написан"
    );
    assert!(
        !dir.path().join("v8project.local.yaml").exists(),
        "и до того, как объявлен местный слой"
    );
}

/// Два ключа об одном адресе — отказ: выбирать за вызывающего раннер не станет.
#[test]
fn init_refuses_two_keys_naming_one_address() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args([
            "init",
            "--connection",
            "File=build/ib",
            "--infobase",
            "Srvr=srv;Ref=erp",
        ])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2));
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(reported.contains("--connection"), "{reported}");
    assert!(reported.contains("--infobase"), "{reported}");
}

/// Отказ существующего слоя называет тот ключ, которым адрес передали.
#[test]
fn an_existing_origin_is_not_replaced_and_the_refusal_names_the_key_that_was_used() {
    let dir = temp_workspace();
    fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("xml");
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "infobases:\n  origin:\n    connection: 'File=/srv/ib'\n",
    )
    .expect("local");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["init", "--infobase", "Srvr=srv;Ref=erp"])
        .output()
        .expect("run init");

    assert_eq!(output.status.code(), Some(2));
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(reported.contains("--infobase"), "{reported}");
    assert!(!reported.contains("--connection"), "{reported}");
    // Объявленный адрес переживает отказ дословно: отказ на то и отказ, чтобы его не терять.
    assert_eq!(
        fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local"),
        "infobases:\n  origin:\n    connection: 'File=/srv/ib'\n"
    );
}

/// Один генератор пишет все шаблоны проекта: местный слой, опись версий одной базы
/// и замок выгрузки. Повторный запуск ничего не дублирует.
#[test]
fn init_ignores_project_local_files_once() {
    let dir = temp_workspace();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-q", "-b", "main", "."])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git init failed");
    let main = dir.path().join("src").join("configuration");
    fs::create_dir_all(&main).expect("main");
    fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");

    for _ in 0..2 {
        let output = v8_runner_command()
            .current_dir(dir.path())
            .args(["init", "--force"])
            .output()
            .expect("run init");
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
    assert_eq!(
        gitignore,
        "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
    );
}

/// Конфиг во вложенном каталоге, наборы — в `src/…`: `.gitignore` рядом с конфигом
/// до наборов не дотягивается, поэтому шаблоны уходят в корневой `.gitignore`
/// рабочей копии, и ответ называет именно его.
#[test]
fn init_with_a_nested_config_ignores_the_version_file_of_every_source_set() {
    let dir = temp_workspace();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-q", "-b", "main", "."])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git init failed");
    let main = dir.path().join("src").join("configuration");
    fs::create_dir_all(&main).expect("main");
    fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args([
            "--json-message",
            "init",
            "--output",
            "config/v8project.yaml",
        ])
        .output()
        .expect("run init");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let canonical_dir = fs::canonicalize(dir.path()).expect("canonical project dir");
    assert_eq!(
        payload["data"]["gitignore_path"],
        canonical_dir.join(".gitignore").display().to_string()
    );
    assert!(!dir.path().join("config").join(".gitignore").exists());
    for probe in [
        "src/configuration/ConfigDumpInfo.xml",
        "src/configuration/.dump-main.lock",
        "src/configuration/.dump-main.lock.system",
        "config/v8project.local.yaml",
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["check-ignore", "-q", "--no-index", "--", probe])
            .status()
            .expect("run git");
        assert!(status.success(), "{probe} must be ignored");
    }
}
