#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const V8_EXTENSION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExtensionNature";
const V8_EXTERNAL_OBJECTS_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExternalObjectsNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

#[derive(Clone, Copy)]
struct SourceSetSpec<'a> {
    name: &'a str,
    kind: &'a str,
    path: &'a str,
}

fn write_edt_script(path: &Path, calls_log: &Path) {
    let body = format!(
        r#"args="$*"
printf '%s\n' "$args" >> "{}"
mode=""
project=""
config_files=""
base_project_name=""
prev=""
write_native_project() {{
  target="$1"
  name="$2"
  nature="$3"
  base_project="$4"
  mkdir -p "$target/DT-INF" "$target/src/Configuration"
  cat > "$target/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>$name</name>
  <natures>
    <nature>$nature</nature>
  </natures>
</projectDescription>
EOF
  {{
    if [ -n "$base_project" ]; then printf 'Base-Project: %s\n' "$base_project"; fi
    printf 'Manifest-Version: 1.0\nRuntime-Version: {}\n'
  }} > "$target/DT-INF/PROJECT.PMF"
  printf '<Configuration />\n' > "$target/src/Configuration/Configuration.mdo"
  printf 'Procedure Test()\nEndProcedure\n' > "$target/src/Configuration/Module.bsl"
}}
write_external_project() {{
  target="$1"
  name="$2"
  descriptor="$3"
  mkdir -p "$target/DT-INF" "$target/src"
  cat > "$target/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>$name</name>
  <natures>
    <nature>{}</nature>
  </natures>
</projectDescription>
EOF
  printf 'Base-Project: BaseProject\nManifest-Version: 1.0\nRuntime-Version: {}\n' > "$target/DT-INF/PROJECT.PMF"
  cp "$descriptor" "$target/src/root.xml"
}}
read_project_name() {{
  project_file="$1/.project"
  if [ ! -f "$project_file" ]; then
    printf 'Imported'
    return
  fi
  name=$(sed -n 's:.*<name>\([^<][^<]*\)</name>.*:\1:p' "$project_file" | head -n 1)
  if [ -n "$name" ]; then
    printf '%s' "$name"
  else
    printf 'Imported'
  fi
}}
project_is_extension() {{
  project_file="$1/.project"
  [ -f "$project_file" ] && grep -q '{}' "$project_file"
}}
project_is_external() {{
  [ -f "$1/src/root.xml" ]
}}
read_configuration_name() {{
  config_file="$1/Configuration.xml"
  if [ ! -f "$config_file" ]; then
    printf 'Imported'
    return
  fi
  name=$(sed -n 's:.*<Name>\([^<][^<]*\)</Name>.*:\1:p' "$config_file" | head -n 1)
  if [ -n "$name" ]; then
    printf '%s' "$name"
  else
    printf 'Imported'
  fi
}}
configuration_is_extension() {{
  config_file="$1/Configuration.xml"
  [ -f "$config_file" ] && grep -q 'ConfigurationExtensionPurpose\|ObjectBelonging' "$config_file"
}}
for arg in "$@"; do
  if [ "$prev" = "-command" ]; then mode="$arg"; fi
  if [ "$prev" = "--project" ]; then project="$arg"; fi
  if [ "$prev" = "--configuration-files" ]; then config_files="$arg"; fi
  if [ "$prev" = "--base-project-name" ]; then base_project_name="$arg"; fi
  prev="$arg"
done
case "$mode" in
  export)
    mkdir -p "$config_files"
    rm -rf "$config_files"/*
    if project_is_external "$project"; then
      descriptor_name=$(basename "$project")
      cp "$project/src/root.xml" "$config_files/$descriptor_name.xml"
    else
      project_name=$(read_project_name "$project")
      if project_is_extension "$project"; then
        printf '<Configuration><Properties><Name>%s</Name></Properties><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>\n' "$project_name" > "$config_files/Configuration.xml"
      else
        printf '<Configuration><Properties><Name>%s</Name></Properties></Configuration>\n' "$project_name" > "$config_files/Configuration.xml"
      fi
    fi
    ;;
  import)
    if [ -f "$config_files/Configuration.xml" ]; then
      mkdir -p "$project"
      imported_name=$(read_configuration_name "$config_files")
      if configuration_is_extension "$config_files"; then
        if [ "$base_project_name" != "BaseProject" ]; then
          printf 'unexpected base project: %s\n' "$base_project_name" >&2
          exit 23
        fi
        imported_nature="{}"
        imported_base="BaseProject"
      else
        imported_nature="{}"
        imported_base=""
      fi
      write_native_project "$project" "$imported_name" "$imported_nature" "$imported_base"
    else
      mkdir -p "$project"
      for descriptor in "$config_files"/*.xml; do
        if [ ! -f "$descriptor" ]; then continue; fi
        descriptor_name=$(basename "$descriptor" .xml)
        write_external_project "$project/$descriptor_name" "$descriptor_name" "$descriptor"
      done
    fi
    ;;
esac
exit 0"#,
        calls_log.display(),
        EDT_RUNTIME_VERSION,
        V8_EXTERNAL_OBJECTS_NATURE,
        EDT_RUNTIME_VERSION,
        V8_EXTENSION_NATURE,
        V8_EXTENSION_NATURE,
        V8_CONFIGURATION_NATURE
    );
    write_script(path, &body);
}

fn write_path_named_edt_import_script(path: &Path, calls_log: &Path) {
    let body = format!(
        r#"args="$*"
printf '%s\n' "$args" >> "{}"
mode=""
project=""
config_files=""
base_project_name=""
prev=""
write_native_project() {{
  target="$1"
  name="$2"
  nature="$3"
  base_project="$4"
  mkdir -p "$target/DT-INF" "$target/src/Configuration"
  cat > "$target/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>$name</name>
  <natures>
    <nature>$nature</nature>
  </natures>
</projectDescription>
EOF
  {{
    if [ -n "$base_project" ]; then printf 'Base-Project: %s\n' "$base_project"; fi
    printf 'Manifest-Version: 1.0\nRuntime-Version: {}\n'
  }} > "$target/DT-INF/PROJECT.PMF"
  printf '<Configuration />\n' > "$target/src/Configuration/Configuration.mdo"
}}
write_external_project() {{
  target="$1"
  name="$2"
  descriptor="$3"
  mkdir -p "$target/DT-INF" "$target/src"
  cat > "$target/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>$name</name>
  <natures>
    <nature>{}</nature>
  </natures>
</projectDescription>
EOF
  printf 'Base-Project: BaseProject\nManifest-Version: 1.0\nRuntime-Version: {}\n' > "$target/DT-INF/PROJECT.PMF"
  cp "$descriptor" "$target/src/root.xml"
}}
configuration_is_extension() {{
  config_file="$1/Configuration.xml"
  [ -f "$config_file" ] && grep -q 'ConfigurationExtensionPurpose\|ObjectBelonging' "$config_file"
}}
for arg in "$@"; do
  if [ "$prev" = "-command" ]; then mode="$arg"; fi
  if [ "$prev" = "--project" ]; then project="$arg"; fi
  if [ "$prev" = "--configuration-files" ]; then config_files="$arg"; fi
  if [ "$prev" = "--base-project-name" ]; then base_project_name="$arg"; fi
  prev="$arg"
done
case "$mode" in
  import)
    if [ -f "$config_files/Configuration.xml" ]; then
      mkdir -p "$project"
      imported_name=$(basename "$project")
      if configuration_is_extension "$config_files"; then
        if [ "$base_project_name" != "configuration" ]; then
          printf 'unexpected base project: %s\n' "$base_project_name" >&2
          exit 23
        fi
        write_native_project "$project" "$imported_name" "{}" "$base_project_name"
      else
        write_native_project "$project" "$imported_name" "{}" ""
      fi
    else
      mkdir -p "$project"
      for descriptor in "$config_files"/*.xml; do
        if [ ! -f "$descriptor" ]; then continue; fi
        descriptor_name=$(basename "$descriptor" .xml)
        write_external_project "$project/$descriptor_name" "$descriptor_name" "$descriptor"
      done
    fi
    ;;
esac
exit 0"#,
        calls_log.display(),
        EDT_RUNTIME_VERSION,
        V8_EXTERNAL_OBJECTS_NATURE,
        EDT_RUNTIME_VERSION,
        V8_EXTENSION_NATURE,
        V8_CONFIGURATION_NATURE
    );
    write_script(path, &body);
}

fn write_config(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    edt_path: &Path,
    format: &str,
    source_sets: &[SourceSetSpec<'_>],
    platform_version: Option<&str>,
) {
    let mut config = format!(
        "workPath: '{}'\nformat: {format}\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n",
        work_path.display(),
    );
    for source_set in source_sets {
        config.push_str(&format!(
            "  - name: {}\n    type: {}\n    path: {}\n",
            source_set.name, source_set.kind, source_set.path
        ));
    }
    config.push_str("tools:\n");
    if let Some(version) = platform_version {
        config.push_str(&format!("  platform:\n    version: '{version}'\n"));
    }
    config.push_str(&format!(
        "  edt_cli:\n    path: '{}'\n    interactive-mode: false\n",
        edt_path.display()
    ));
    fs::write(path, config).expect("config");
}

fn write_live_workspace_lock(work_path: &Path, command: &str) {
    let canonical_work = fs::canonicalize(work_path).expect("canonical work");
    let lock_owner = "integration-test-lock-owner";
    let started_at = chrono::Utc::now().to_rfc3339();

    fs::write(
        canonical_work.join(".v8-runner.workspace.lock"),
        serde_json::json!({
            "tool": "v8-runner",
            "pid": std::process::id(),
            "owner_id": lock_owner,
            "created_at": started_at,
        })
        .to_string(),
    )
    .expect("workspace lock");
    fs::write(
        canonical_work.join(".v8-runner.workspace.lock.json"),
        serde_json::json!({
            "pid": std::process::id(),
            "lock_owner": lock_owner,
            "command": command,
            "started_at": started_at,
            "canonical_work_path": canonical_work,
        })
        .to_string(),
    )
    .expect("workspace lock sidecar");
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
    let config_path = base_path.join("v8project.yaml");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let calls_log = dir.path().join("edt-calls.log");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_edt_script(&edt_cli_path, &calls_log);

    (
        dir,
        config_path,
        base_path,
        work_path,
        edt_cli_path,
        calls_log,
    )
}

fn write_designer_source(path: &Path, project_name: &str, is_extension: bool) {
    fs::create_dir_all(path).expect("designer source");
    let descriptor = if is_extension {
        format!(
            "<Configuration><Properties><Name>{project_name}</Name></Properties><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>\n"
        )
    } else {
        format!(
            "<Configuration><Properties><Name>{project_name}</Name></Properties></Configuration>\n"
        )
    };
    fs::write(path.join("Configuration.xml"), descriptor).expect("xml");
}

fn write_designer_external_source(path: &Path, names: &[&str]) {
    fs::create_dir_all(path).expect("designer external source");
    for name in names {
        fs::write(
            path.join(format!("{name}.xml")),
            format!(
                "<ExternalDataProcessor><Properties><Name>{name}</Name></Properties></ExternalDataProcessor>\n"
            ),
        )
        .expect("xml");
    }
}

fn write_edt_source(path: &Path, name: &str, descriptor_xml: &str) {
    fs::create_dir_all(path).expect("edt source");
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
    let is_extension = descriptor_xml.contains("ConfigurationExtensionPurpose")
        || descriptor_xml.contains("ObjectBelonging");
    let nature = if is_extension {
        V8_EXTENSION_NATURE
    } else {
        V8_CONFIGURATION_NATURE
    };
    let base_project_line = if is_extension {
        "Base-Project: BaseProject\n"
    } else {
        ""
    };
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>{nature}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!(
            "{base_project_line}Manifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"
        ),
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

fn write_edt_external_project(path: &Path, name: &str) {
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>{V8_EXTERNAL_OBJECTS_NATURE}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!(
            "Base-Project: BaseProject\nManifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"
        ),
    )
    .expect("manifest");
    fs::write(
        path.join("src").join("root.xml"),
        format!(
            "<ExternalDataProcessor><Properties><Name>{name}</Name></Properties></ExternalDataProcessor>\n"
        ),
    )
    .expect("descriptor");
}

fn assert_native_edt_project(path: &Path) {
    assert!(path.join(".project").exists());
    assert!(path.join("DT-INF").join("PROJECT.PMF").exists());
    assert!(path.join("src/Configuration/Configuration.mdo").exists());
}

fn assert_native_edt_external_project(path: &Path) {
    assert!(path.join(".project").exists());
    assert!(path.join("DT-INF").join("PROJECT.PMF").exists());
    assert!(path.join("src").join("root.xml").exists());
}

#[test]
fn convert_dry_run_plans_every_source_set_without_dispatching_the_edt_cli() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: "main",
            },
            SourceSetSpec {
                name: "ext-sales",
                kind: "EXTENSION",
                path: "ext-sales",
            },
        ],
        Some("8.3.24"),
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_designer_source(&base_path.join("ext-sales"), "SalesExtension", true);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--dry-run",
        ])
        .output()
        .expect("run convert preview");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false);
    assert_eq!(data["direction"], "DESIGNER_TO_EDT");
    let planned: Vec<&str> = data["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .map(|output| output["source_set"].as_str().expect("source set"))
        .collect();
    assert_eq!(planned, vec!["main", "ext-sales"]);
    assert!(!calls_log.exists(), "preview must not dispatch the EDT CLI");
    // The preview names the targets; it must not create them.
    for output in data["outputs"].as_array().expect("outputs") {
        let target = Path::new(output["target_path"].as_str().expect("target"));
        assert!(!target.exists(), "{}", target.display());
    }
    assert!(!work_path.join("convert").join("edt-workspace").exists());
}

#[test]
fn convert_without_source_set_processes_all_source_sets_into_work_path_out() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: "main",
            },
            SourceSetSpec {
                name: "ext-sales",
                kind: "EXTENSION",
                path: "ext-sales",
            },
        ],
        Some("8.3.24"),
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_designer_source(&base_path.join("ext-sales"), "SalesExtension", true);
    let stale_output = work_path.join("convert/out/main/edt/stale.txt");
    fs::create_dir_all(stale_output.parent().expect("parent")).expect("stale dir");
    fs::write(&stale_output, "stale").expect("stale file");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "convert");
    assert_eq!(payload["data"]["direction"], "DESIGNER_TO_EDT");
    assert_eq!(payload["data"]["scope"], "ALL");
    // У перевода между EDT и XML выбора исполнителя нет, и квитанции тоже.
    assert!(payload["data"].get("provider").is_none(), "{payload}");
    assert_eq!(
        payload["data"]["outputs"]
            .as_array()
            .expect("outputs")
            .len(),
        2
    );
    assert_eq!(payload["data"]["outputs"][0]["source_set"], "main");
    assert_eq!(payload["data"]["outputs"][1]["source_set"], "ext-sales");

    let main_target = work_path.join("convert/out/main/edt");
    let extension_target = work_path.join("convert/out/ext-sales/edt");
    assert_native_edt_project(&main_target);
    assert_native_edt_project(&extension_target);
    assert!(!stale_output.exists());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command import").count(), 2);
    assert!(calls.contains("--version 8.3.24"));
    assert!(calls.contains("--base-project-name BaseProject"));
    assert!(!calls.contains("--build true"));
}

#[test]
fn convert_single_source_set_uses_inferred_edt_to_designer_direction() {
    let (_dir, config_path, base_path, work_path, _edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &_edt_cli_path,
        "EDT",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: "main",
            },
            SourceSetSpec {
                name: "ext-sales",
                kind: "EXTENSION",
                path: "ext-sales",
            },
        ],
        None,
    );
    write_edt_source(
        &base_path.join("main"),
        "MainConfiguration",
        "<Configuration />",
    );
    write_edt_source(
        &base_path.join("ext-sales"),
        "SalesExtension",
        "<Configuration><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "convert",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Convert completed successfully"));
    assert!(stdout.contains("direction: edt-to-designer"));
    assert!(stdout.contains("scope: source-set main"));
    assert!(stdout.contains(
        work_path
            .join("convert/out/main/designer")
            .display()
            .to_string()
            .as_str()
    ));

    let target = work_path.join("convert/out/main/designer");
    assert!(target.join("Configuration.xml").exists());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command export").count(), 1);
    assert!(calls.contains(base_path.join("main").display().to_string().as_str()));
    assert!(!calls.contains(base_path.join("ext-sales").display().to_string().as_str()));
}

#[test]
fn convert_single_extension_source_set_infers_base_project_name_from_configuration_source() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: "main",
            },
            SourceSetSpec {
                name: "ext-sales",
                kind: "EXTENSION",
                path: "ext-sales",
            },
        ],
        Some("8.3.24"),
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_designer_source(&base_path.join("ext-sales"), "SalesExtension", true);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "--log-level",
            "warn",
            "convert",
            "--source-set",
            "ext-sales",
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("◌ convert: base project import"));
    assert!(!stdout.contains("started_at: "));
    assert!(stdout.contains("[EDT] importing Designer files for base project name"));
    assert!(
        stdout
            .find("convert: base project import")
            .expect("base project stage")
            < stdout
                .find("convert: designer import")
                .expect("extension import stage")
    );
    assert!(stdout.contains("● Convert completed successfully"));

    let target = work_path.join("convert/out/ext-sales/edt");
    assert_native_edt_project(&target);

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command import").count(), 2);
    assert!(calls.contains(base_path.join("main").display().to_string().as_str()));
    assert!(calls.contains("--base-project-name BaseProject"));
    assert!(calls.contains("--version 8.3.24"));
}

#[test]
fn convert_unknown_source_set_json_keeps_convert_command_identity_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--source-set",
            "missing",
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("unknown source-set 'missing'"));
}

#[test]
fn convert_workspace_lock_conflict_answers_workspace_busy_after_valid_preflight() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "convert",
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ERROR: workspace busy: cannot start convert"),
        "stderr:\n{stderr}"
    );
}

/// Утилиты нет — отказ рода `environment` с кодом `environment_unavailable` и выходом 2:
/// поставьте её, и заработает. Род `platform` оставлен за сбоем самой платформы.
#[test]
fn convert_without_the_edt_cli_answers_an_environment_failure() {
    let (dir, config_path, base_path, work_path, _edt_cli_path, calls_log) = setup_project();
    let missing_edt_cli = dir.path().join("absent").join("1cedtcli");
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &missing_edt_cli,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    let empty_path = dir.path().join("no-tools");
    fs::create_dir_all(&empty_path).expect("empty PATH dir");

    let output = v8_runner_command()
        .env("PATH", &empty_path)
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
        ])
        .output()
        .expect("run convert");

    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(output.status.code(), Some(2), "{payload}");
    assert_eq!(payload["ok"], false, "{payload}");
    assert_eq!(payload["command"], "convert", "{payload}");
    assert_eq!(payload["error"]["kind"], "environment", "{payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{payload}"
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("was not found")),
        "the refusal must name the missing utility: {payload}"
    );
    assert!(!calls_log.exists(), "no EDT CLI ran");
}

#[test]
fn convert_external_edt_source_set_preserves_all_exported_descriptors() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "EDT",
        &[SourceSetSpec {
            name: "processors",
            kind: "EXTERNAL_DATA_PROCESSORS",
            path: "processors",
        }],
        None,
    );
    write_edt_external_project(&base_path.join("processors/processor-a"), "ProcessorA");
    write_edt_external_project(&base_path.join("processors/processor-b"), "ProcessorB");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--source-set",
            "processors",
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "convert");
    assert_eq!(payload["data"]["direction"], "EDT_TO_DESIGNER");
    assert_eq!(payload["data"]["scope"], "SINGLE");

    let target = work_path.join("convert/out/processors/designer");
    assert!(target.join("processor-a.xml").exists());
    assert!(target.join("processor-b.xml").exists());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command export").count(), 2);
}

#[test]
fn convert_external_designer_source_set_does_not_require_configuration_source_set() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "processors",
            kind: "EXTERNAL_DATA_PROCESSORS",
            path: "processors",
        }],
        None,
    );
    write_designer_external_source(
        &base_path.join("processors"),
        &["processor-a", "processor-b"],
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "convert");
    assert_eq!(payload["data"]["direction"], "DESIGNER_TO_EDT");
    assert_eq!(payload["data"]["scope"], "ALL");

    let target = work_path.join("convert/out/processors/edt");
    assert_native_edt_external_project(&target.join("processor-a"));
    assert_native_edt_external_project(&target.join("processor-b"));

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command import").count(), 1);
    assert!(!calls.contains("--base-project-name"));
}

#[test]
fn convert_output_root_mirrors_source_set_layout_and_stabilizes_edt_project_names() {
    let dir = temp_workspace();
    let base_path = dir.path().join("designer");
    let work_path = dir.path().join("work");
    let output_root = dir.path().join("edt");
    let config_path = base_path.join("v8project.yaml");
    let edt_cli_path = dir.path().join("edt-cli").join("1cedtcli");
    let calls_log = dir.path().join("edt-calls.log");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_path_named_edt_import_script(&edt_cli_path, &calls_log);
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "configuration",
                kind: "CONFIGURATION",
                path: "configuration",
            },
            SourceSetSpec {
                name: "extension",
                kind: "EXTENSION",
                path: "extension",
            },
            SourceSetSpec {
                name: "processors",
                kind: "EXTERNAL_DATA_PROCESSORS",
                path: "external/processor",
            },
        ],
        None,
    );
    write_designer_source(&base_path.join("configuration"), "BaseProject", false);
    write_designer_source(&base_path.join("extension"), "SalesExtension", true);
    write_designer_external_source(
        &base_path.join("external/processor"),
        &["processor-a", "processor-b"],
    );
    let stale_file = output_root.join("configuration").join("stale.txt");
    fs::create_dir_all(stale_file.parent().expect("stale parent")).expect("stale dir");
    fs::write(&stale_file, "stale").expect("stale");
    // Каталог вывода, названный человеком, вне системы контроля версий заменить нельзя.
    support::commit_sources(&output_root);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &output_root.display().to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "convert");
    assert_eq!(payload["data"]["direction"], "DESIGNER_TO_EDT");
    assert_eq!(payload["data"]["scope"], "ALL");

    let configuration_target = output_root.join("configuration");
    let extension_target = output_root.join("extension");
    let processors_target = output_root.join("external").join("processor");
    assert_native_edt_project(&configuration_target);
    assert_native_edt_project(&extension_target);
    assert_native_edt_external_project(&processors_target.join("processor-a"));
    assert_native_edt_external_project(&processors_target.join("processor-b"));
    assert!(!stale_file.exists());

    let configuration_project =
        fs::read_to_string(configuration_target.join(".project")).expect("configuration project");
    let extension_project =
        fs::read_to_string(extension_target.join(".project")).expect("extension project");
    let extension_manifest = fs::read_to_string(extension_target.join("DT-INF/PROJECT.PMF"))
        .expect("extension manifest");
    assert!(configuration_project.contains("<name>configuration</name>"));
    assert!(extension_project.contains("<name>extension</name>"));
    assert!(extension_manifest.contains("Base-Project: configuration"));

    assert_eq!(
        payload["data"]["outputs"][0]["target_path"],
        configuration_target.display().to_string()
    );
    assert_eq!(
        payload["data"]["outputs"][1]["target_path"],
        extension_target.display().to_string()
    );
    assert_eq!(
        payload["data"]["outputs"][2]["target_path"],
        processors_target.display().to_string()
    );

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.matches("-command import").count(), 3);
    assert!(calls.contains("--base-project-name configuration"));
}

#[test]
fn convert_output_root_rejects_source_overlap_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &base_path.display().to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("overlaps source-set 'main' path"));
}

#[test]
fn convert_single_source_output_rejects_unselected_source_set_overlap() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: "main",
            },
            SourceSetSpec {
                name: "ext",
                kind: "EXTENSION",
                path: "ext",
            },
        ],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_designer_source(&base_path.join("ext"), "SalesExtension", true);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--source-set",
            "main",
            "--output",
            &base_path.join("ext").display().to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("overlaps source-set 'ext' path"));
}

#[test]
fn convert_output_root_rejects_base_path_child_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &base_path.join("generated").display().to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("must not be inside project base path"));
}

#[test]
fn convert_output_root_rejects_work_path_child_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &work_path
                .join("convert")
                .join("edt-workspace")
                .display()
                .to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("must not be inside workPath"));
}

#[test]
fn convert_output_root_rejects_filesystem_root_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    write_live_workspace_lock(&work_path, "convert");
    let root_output = std::path::MAIN_SEPARATOR.to_string();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &root_output,
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("filesystem root"));
}

#[test]
fn convert_output_root_rejects_overlapping_targets_before_workspace_lock() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[
            SourceSetSpec {
                name: "main",
                kind: "CONFIGURATION",
                path: ".",
            },
            SourceSetSpec {
                name: "nested-ext",
                kind: "EXTENSION",
                path: "nested",
            },
        ],
        None,
    );
    write_designer_source(&base_path, "BaseProject", false);
    write_designer_source(&base_path.join("nested"), "NestedExtension", true);
    write_live_workspace_lock(&work_path, "convert");
    let output_root = base_path.parent().expect("parent").join("converted");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "--output",
            &output_root.display().to_string(),
        ])
        .output()
        .expect("run convert");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "convert");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("output targets overlap"));
}

/// `convert <SET> --output X` над работой вне учёта: совет — тот же вызов с `--force`, а не
/// урезанный `convert --force`, который потерял бы набор и каталог вывода.
#[test]
fn a_convert_refusal_does_not_offer_a_truncated_command() {
    let (dir, config_path, base_path, work_path, edt_cli_path, _calls_log) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "EDT",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_edt_source(
        &base_path.join("main"),
        "MainConfiguration",
        "<Configuration />",
    );
    let repository = dir.path().join("elsewhere");
    let output_dir = repository.join("designer");
    // Один набор с `--output` кладётся в каталог с именем набора внутри него.
    let target = output_dir.join("main");
    fs::create_dir_all(&target).expect("target dir");
    let vcs = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
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
    fs::write(repository.join("README.md"), "readme\n").expect("readme");
    vcs(&["add", "-A"]);
    vcs(&["commit", "-qm", "readme"]);
    fs::write(target.join("hand-written.xml"), "mine\n").expect("hand-written");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "main",
            "--output",
            &output_dir.display().to_string(),
        ])
        .output()
        .expect("run convert");

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(output.status.code(), Some(2), "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("hand-written.xml"), "{message}");
    assert!(!message.contains("`convert --force`"), "{message}");
    assert!(
        message.contains("repeat the same command with `--force` added"),
        "{message}"
    );
    assert!(target.join("hand-written.xml").is_file());

    // Совет, выполненный буквально: `--force` у `convert` ни с чем не спорит, и тот же вызов
    // с ключом заменяет тот же каталог, а не упирается во второй отказ.
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "main",
            "--output",
            &output_dir.display().to_string(),
            "--force",
        ])
        .output()
        .expect("run convert with --force");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!target.join("hand-written.xml").exists());
    // Согласие называет уничтоженное.
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("discarded on request"), "{message}");
    assert!(message.contains("hand-written.xml"), "{message}");
}

// --- Направления с пакетом (#236) -----------------------------------------------------
//
// Поддельный `ibcmd` записывает каждый вызов, создаёт базу по `infobase create`, пишет пакет
// по ключу `--out=` и раскладывает XML по `config export --file=`. Файл `fail` рядом с ним
// роняет импорт и разбор кодом 17, файл `cancel` присылает раннеру SIGTERM во время импорта.
// Проект базы не объявляет: ни `infobase:` в проектном файле, ни местного слоя.

const IBCMD: &str = r#"root="$(dirname "$0")/.."
printf '%s\n' "$*" >> "$root/ibcmd-calls"
case " $* " in
  *" import "*|*" export "*)
    if [ -f "$root/fail" ]; then exit 17; fi
    if [ -f "$root/cancel" ]; then kill -TERM "$PPID"; sleep 5; fi
    ;;
esac
db=''
previous=''
last=''
for argument in "$@"; do
  case "$argument" in
    --out=*) printf 'package' > "${argument#--out=}" ;;
  esac
  if [ "$previous" = --db-path ]; then db="$argument"; fi
  previous="$argument"
  last="$argument"
done
case " $* " in
  *" create "*) mkdir -p "$db"; : > "$db/1Cv8.1CD" ;;
  *" export "*) mkdir -p "$last"; printf '<Configuration />' > "$last/Configuration.xml" ;;
esac
exit 0"#;

struct PackageProject {
    _dir: tempfile::TempDir,
    /// Канонический путь временного каталога: на macOS `/var` — ссылка на `/private/var`, а
    /// раннер отвечает каноническими путями.
    base: PathBuf,
    root: PathBuf,
}

impl PackageProject {
    /// Наборы `main` (конфигурация), `Sales` (расширение) и `tools` (внешние обработки) в
    /// формате Конфигуратора; `ibcmd` лежит в `platform/bin`, база не объявлена.
    fn new() -> Self {
        let dir = temp_workspace();
        let base = fs::canonicalize(dir.path()).expect("canonical temp dir");
        let root = base.join("project");
        write_designer_source(&root.join("src/cf"), "Main", false);
        write_designer_source(&root.join("src/sales"), "Sales", true);
        write_designer_external_source(&root.join("src/tools"), &["Tool"]);
        fs::write(
            root.join("v8project.yaml"),
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: tools\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src/tools\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\n  - name: Sales\n    type: EXTENSION\n    path: src/sales\ntools:\n  platform:\n    path: platform\n",
        )
        .expect("project file");
        write_script(&root.join("platform/bin/ibcmd"), IBCMD);
        Self {
            _dir: dir,
            base,
            root,
        }
    }

    fn run(&self, args: &[&str]) -> (std::process::Output, Value) {
        let output = v8_runner_command()
            .current_dir(&self.root)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run convert");
        let envelope = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "one json document expected ({error}):\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output, envelope)
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("platform/ibcmd-calls"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    fn throwaway_root(&self) -> PathBuf {
        self.root.join("work/temp/throwaway-infobases")
    }

    fn bases_left(&self) -> usize {
        fs::read_dir(self.throwaway_root())
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
}

/// Набор в пакет: `ibcmd` создаёт временную базу под `workPath` со своим `--data` и собирает
/// пакет `config import --out` — база проекта не нужна и не объявлена; база убирается, а
/// квитанция называет исполнителя.
#[test]
fn convert_a_set_to_a_package_builds_it_with_ibcmd_in_a_throwaway_base() {
    let project = PackageProject::new();

    let (output, envelope) = project.run(&["convert", "Sales", "--to", "package"]);

    assert!(output.status.success(), "{envelope}");
    let data = &envelope["data"];
    assert_eq!(data["direction"], "DESIGNER_TO_PACKAGE", "{envelope}");
    assert_eq!(data["provider_dispatched"], true, "{envelope}");
    assert_eq!(data["provider"]["selected"], "ibcmd", "{envelope}");
    assert_eq!(data["provider"]["origin"]["kind"], "default", "{envelope}");
    let target = project.root.join("work/convert/out/packages/Sales.cfe");
    assert_eq!(fs::read_to_string(&target).expect("package"), "package");
    assert_eq!(
        data["outputs"][0]["target_path"],
        target.display().to_string()
    );

    let calls = project.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let throwaway = project.throwaway_root().display().to_string();
    for call in &calls {
        assert!(
            call.starts_with(&format!("infobase --data {throwaway}")),
            "every call carries its own data directory under the throwaway root: {call}"
        );
        assert!(!call.contains("ibcmd-data"), "{call}");
    }
    assert!(calls[0].ends_with(" create"), "{calls:?}");
    assert!(calls[1].contains(" config import --out="), "{calls:?}");
    assert!(
        calls[1].ends_with(&project.root.join("src/sales").display().to_string()),
        "{calls:?}"
    );
    assert_eq!(project.bases_left(), 0, "the throwaway base is removed");
}

/// Без набора `--to package` берёт пакеты конфигурации проекта — основную конфигурацию и
/// расширения — в одной временной базе; набор внешних обработок в обход не входит.
#[test]
fn convert_without_a_set_to_a_package_takes_the_configuration_packages() {
    let project = PackageProject::new();

    let (output, envelope) = project.run(&["convert", "--to", "package"]);

    assert!(output.status.success(), "{envelope}");
    let sets: Vec<&str> = envelope["data"]["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .map(|output| output["source_set"].as_str().expect("set"))
        .collect();
    assert_eq!(sets, ["main", "Sales"], "{envelope}");
    let out = project.root.join("work/convert/out/packages");
    assert!(out.join("main.cf").is_file());
    assert!(out.join("Sales.cfe").is_file());
    assert!(!out.join("tools").exists());
    let calls = project.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.ends_with(" create"))
            .count(),
        1,
        "one throwaway base serves the run: {calls:?}"
    );
    assert_eq!(project.bases_left(), 0);
}

/// Набор внешних файлов пакета конфигурации не называет: отказ до замка и до платформы.
#[test]
fn convert_an_external_set_to_a_package_is_refused() {
    let project = PackageProject::new();

    let (output, envelope) = project.run(&["convert", "tools", "--to", "package"]);

    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    assert!(
        envelope["error"]["next"].to_string().contains("make"),
        "the refusal names make as the way out: {envelope}"
    );
    assert!(project.calls().is_empty());
}

/// Файл пакета в XML: `ibcmd` разбирает его `config export --file` во временной базе
/// раннера со своим `--data`, каталог `--output` получает XML, база убирается.
#[test]
fn convert_a_package_file_to_xml_exports_it_in_a_throwaway_base() {
    let project = PackageProject::new();
    let package = project.base.join("main.cf");
    fs::write(&package, "package").expect("package file");
    let target = project.base.join("xml");

    let (output, envelope) = project.run(&[
        "convert",
        &package.display().to_string(),
        "--to",
        "xml",
        "--output",
        &target.display().to_string(),
    ]);

    assert!(output.status.success(), "{envelope}");
    let data = &envelope["data"];
    assert_eq!(data["direction"], "PACKAGE_TO_DESIGNER", "{envelope}");
    assert_eq!(data["scope"], "PACKAGE", "{envelope}");
    assert_eq!(data["provider"]["selected"], "ibcmd", "{envelope}");
    assert!(data["outputs"][0].get("source_set").is_none(), "{envelope}");
    assert!(target.join("Configuration.xml").is_file(), "{envelope}");

    let calls = project.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let throwaway = project.throwaway_root().display().to_string();
    assert!(calls[0].ends_with(" create"), "{calls:?}");
    assert!(
        calls[1].starts_with(&format!("config --data {throwaway}")),
        "{calls:?}"
    );
    assert!(
        calls[1].contains(&format!(" export --file={}", package.display())),
        "{calls:?}"
    );
    assert_eq!(project.bases_left(), 0);
}

/// Без `--to` файл пакета идёт в XML и по умолчанию ложится под `workPath`.
#[test]
fn convert_a_package_file_without_to_goes_to_xml_under_work_path() {
    let project = PackageProject::new();
    let package = project.base.join("ext.cfe");
    fs::write(&package, "package").expect("package file");

    let (output, envelope) = project.run(&["convert", &package.display().to_string()]);

    assert!(output.status.success(), "{envelope}");
    assert_eq!(envelope["data"]["direction"], "PACKAGE_TO_DESIGNER");
    // Каталог назван файлом целиком: `ext.cf` и `ext.cfe` его не делят, а каталог набора
    // `ext` под `convert/out` с ним не совпадает.
    assert!(project
        .root
        .join("work/convert/out/from-package/ext.cfe/Configuration.xml")
        .is_file());
}

/// Без `ibcmd` направлению с пакетом исполнить некому: отказ рода `environment`, квитанция
/// называет пропущенного; `ibcmd-rs` строки `convert` не имеет до замера (#413).
#[test]
fn convert_a_package_direction_without_ibcmd_answers_an_environment_failure() {
    let project = PackageProject::new();
    fs::remove_file(project.root.join("platform/bin/ibcmd")).expect("remove ibcmd");
    let empty_path = project.base.join("no-tools");
    fs::create_dir_all(&empty_path).expect("empty PATH");

    let output = v8_runner_command()
        .current_dir(&project.root)
        .env("PATH", &empty_path)
        .args(["--json-message", "convert", "main", "--to", "package"])
        .output()
        .expect("run convert");
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("json");

    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "environment", "{envelope}");
    assert_eq!(
        envelope["error"]["code"], "environment_unavailable",
        "{envelope}"
    );
    let receipt = &envelope["data"]["provider"];
    assert!(receipt["selected"].is_null(), "{envelope}");
    let skipped: Vec<&str> = receipt["skipped"]
        .as_array()
        .expect("skipped")
        .iter()
        .map(|entry| entry["provider"].as_str().expect("provider"))
        .collect();
    assert_eq!(skipped, ["ibcmd"], "{envelope}");
    assert_eq!(envelope["data"]["provider_dispatched"], false, "{envelope}");
}

/// Превью направления с пакетом находит `ibcmd` и называет цели, но ничего не запускает и
/// временной базы не создаёт.
#[test]
fn convert_a_package_preview_dispatches_nothing() {
    let project = PackageProject::new();

    let (output, envelope) = project.run(&["convert", "--to", "package", "--dry-run"]);

    assert!(output.status.success(), "{envelope}");
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false, "{envelope}");
    assert_eq!(data["provider"]["selected"], "ibcmd", "{envelope}");
    assert_eq!(data["outputs"].as_array().expect("outputs").len(), 2);
    assert!(project.calls().is_empty());
    assert!(!project.root.join("work/temp").exists());
    assert!(!project.root.join("work/convert").exists());
}

/// `--to` в тот формат, в котором исходники уже лежат, и файл пакета не в XML — отказ
/// валидации до замка и до платформы, с выходом в тексте.
#[test]
fn convert_refuses_a_direction_that_is_not_a_conversion() {
    let project = PackageProject::new();
    let package = project.base.join("main.cf");
    fs::write(&package, "package").expect("package file");
    let package = package.display().to_string();

    for (args, way_out) in [
        (vec!["convert", "--to", "xml"], "--to edt or --to package"),
        (vec!["convert", &package, "--to", "edt"], "--to xml"),
        (vec!["convert", &package, "--to", "package"], "--to xml"),
    ] {
        let (output, envelope) = project.run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {envelope}");
        assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(way_out)),
            "{args:?}: {envelope}"
        );
    }
    assert!(project.calls().is_empty());
    assert!(!project.root.join("work/temp").exists());
    assert!(!project.root.join("work/convert").exists());
}

/// `convert` базу проекта не выбирает: проект без базы переводит, а `--infobase` у него —
/// отказ валидации.
#[test]
fn convert_needs_no_infobase_and_refuses_the_infobase_key() {
    let project = PackageProject::new();

    let (output, envelope) = project.run(&["convert", "main", "--to", "package", "--dry-run"]);
    assert!(output.status.success(), "{envelope}");

    let (output, envelope) =
        project.run(&["convert", "main", "--to", "package", "--infobase", "origin"]);
    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    assert!(project.calls().is_empty());
}

/// Исходники формата EDT в пакет: сперва `1cedtcli` переводит набор в XML в каталог
/// временной базы, затем `ibcmd` собирает пакет из этого XML `config import --out`.
#[test]
fn convert_an_edt_set_to_a_package_goes_through_xml_in_the_throwaway_base() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, calls_log) = setup_project();
    write_edt_source(
        &base_path.join("main"),
        "MainConfiguration",
        "<Configuration />",
    );
    write_script(&base_path.join("platform/bin/ibcmd"), IBCMD);
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\ntools:\n  platform:\n    path: platform\n  edt_cli:\n    path: '{}'\n    interactive-mode: false\n",
            work_path.display(),
            edt_cli_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "convert",
            "main",
            "--to",
            "package",
        ])
        .output()
        .expect("run convert");

    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "json ({error}): {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(output.status.success(), "{envelope}");
    assert_eq!(
        envelope["data"]["direction"], "EDT_TO_PACKAGE",
        "{envelope}"
    );
    assert!(work_path.join("convert/out/packages/main.cf").is_file());
    let edt_calls = fs::read_to_string(&calls_log).expect("edt calls");
    assert!(edt_calls.contains("-command export"), "{edt_calls}");
    let ibcmd_calls = fs::read_to_string(base_path.join("platform/ibcmd-calls")).expect("calls");
    let import = ibcmd_calls
        .lines()
        .find(|call| call.contains(" config import --out="))
        .expect("import call");
    let throwaway = work_path.join("temp/throwaway-infobases");
    assert!(
        import.contains(&throwaway.display().to_string()) && import.ends_with("/xml/main"),
        "the package is built from the XML the EDT CLI wrote into the throwaway base: {import}"
    );
}

impl PackageProject {
    fn marker(&self, name: &str) {
        fs::write(self.root.join("platform").join(name), "").expect("marker");
    }
}

/// `ibcmd` выбран и упал: отказ рода `platform` несёт квитанцию, временная база убрана,
/// цель не тронута.
#[test]
fn convert_a_failed_ibcmd_step_removes_the_base_and_keeps_the_receipt() {
    let project = PackageProject::new();
    project.marker("fail");

    let (output, envelope) = project.run(&["convert", "main", "--to", "package"]);

    assert!(!output.status.success(), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "platform", "{envelope}");
    assert_eq!(
        envelope["data"]["provider"]["selected"], "ibcmd",
        "{envelope}"
    );
    assert_eq!(envelope["data"]["provider_dispatched"], true, "{envelope}");
    assert!(
        project
            .calls()
            .iter()
            .any(|call| call.starts_with("infobase ") && call.ends_with(" create")),
        "the throwaway base was created before the step: {:?}",
        project.calls()
    );
    assert_eq!(project.bases_left(), 0, "the throwaway base is removed");
    assert!(!project
        .root
        .join("work/convert/out/packages/main.cf")
        .exists());
}

/// Отмена во время работы `ibcmd` останавливает прогон, и временная база убирается.
#[test]
fn convert_a_cancelled_run_removes_the_throwaway_base() {
    let project = PackageProject::new();
    project.marker("cancel");

    let (output, envelope) = project.run(&["convert", "main", "--to", "package"]);

    assert!(!output.status.success(), "{envelope}");
    assert_eq!(envelope["error"]["code"], "cancelled", "{envelope}");
    assert!(
        project
            .calls()
            .iter()
            .any(|call| call.starts_with("infobase ") && call.ends_with(" create")),
        "the throwaway base was created before the step: {:?}",
        project.calls()
    );
    assert_eq!(project.bases_left(), 0, "the throwaway base is removed");
    assert!(!project
        .root
        .join("work/convert/out/packages/main.cf")
        .exists());
}

/// `--output` файла пакета, лежащий в каталоге набора, под `basePath` или под `workPath`, —
/// отказ до замка и до платформы.
#[test]
fn convert_a_package_file_output_inside_the_project_is_refused() {
    let project = PackageProject::new();
    let package = project.base.join("main.cf");
    fs::write(&package, "package").expect("package file");
    for output in [
        project.root.join("src/cf/xml"),
        project.root.join("xml"),
        project.root.join("work/xml"),
    ] {
        let (status, envelope) = project.run(&[
            "convert",
            &package.display().to_string(),
            "--output",
            &output.display().to_string(),
        ]);
        assert_eq!(status.status.code(), Some(2), "{envelope}");
        assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
        assert!(!output.exists(), "{}", output.display());
    }
    assert!(project.calls().is_empty());
}

/// `--output` у `--to package` читается как у `make`: без набора путь файла — отказ с
/// выходом `next`, с набором — сам файл пакета или каталог для него.
#[test]
fn convert_to_package_reads_the_output_as_make_does() {
    let project = PackageProject::new();
    let out = project.base.join("out");

    let (output, envelope) = project.run(&[
        "convert",
        "--to",
        "package",
        "--output",
        &out.join("all.cf").display().to_string(),
    ]);
    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    assert!(
        envelope["error"]["next"].to_string().contains("main"),
        "{envelope}"
    );

    let file = out.join("sales.cfe");
    let (output, envelope) = project.run(&[
        "convert",
        "Sales",
        "--to",
        "package",
        "--output",
        &file.display().to_string(),
    ]);
    assert!(output.status.success(), "{envelope}");
    assert!(file.is_file(), "{envelope}");

    let (output, envelope) = project.run(&[
        "convert",
        "Sales",
        "--to",
        "package",
        "--output",
        &out.join("sales.cf").display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a .cf for an extension: {envelope}"
    );

    let (output, envelope) = project.run(&[
        "convert",
        "main",
        "--to",
        "package",
        "--output",
        &out.join("dir").display().to_string(),
    ]);
    assert!(output.status.success(), "{envelope}");
    assert!(out.join("dir/main.cf").is_file(), "{envelope}");
}

/// Таблица направлений без файла пакета: `--to` с направлением по умолчанию даёт его же, а
/// `--to edt` у наборов формата EDT — отказ.
#[test]
fn convert_to_names_the_default_direction_explicitly() {
    let (_dir, config_path, base_path, work_path, edt_cli_path, _calls) = setup_project();
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "DESIGNER",
        &[SourceSetSpec {
            name: "main",
            kind: "CONFIGURATION",
            path: "main",
        }],
        None,
    );
    write_designer_source(&base_path.join("main"), "BaseProject", false);
    let run = |args: &[&str]| -> (std::process::Output, Value) {
        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
            ])
            .args(args)
            .output()
            .expect("run convert");
        let envelope = serde_json::from_slice(&output.stdout).expect("json");
        (output, envelope)
    };
    let (output, envelope) = run(&["convert", "--to", "edt", "--dry-run"]);
    assert!(output.status.success(), "{envelope}");
    assert_eq!(
        envelope["data"]["direction"], "DESIGNER_TO_EDT",
        "{envelope}"
    );

    write_config(
        &config_path,
        &base_path,
        &work_path,
        &edt_cli_path,
        "EDT",
        &[SourceSetSpec {
            name: "edt-main",
            kind: "CONFIGURATION",
            path: "edt-main",
        }],
        None,
    );
    write_edt_source(
        &base_path.join("edt-main"),
        "MainConfiguration",
        "<Configuration />",
    );
    let (output, envelope) = run(&["convert", "--to", "xml", "--dry-run"]);
    assert!(output.status.success(), "{envelope}");
    assert_eq!(
        envelope["data"]["direction"], "EDT_TO_DESIGNER",
        "{envelope}"
    );
    let (output, envelope) = run(&["convert", "--to", "edt"]);
    assert_eq!(output.status.code(), Some(2), "{envelope}");
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("--to xml or --to package")),
        "{envelope}"
    );
}
