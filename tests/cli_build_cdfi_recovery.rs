#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const BASELINE: &[u8] = include_bytes!("fixtures/designer/configuration/ConfigDumpInfo.xml");
const PLATFORM_OUTPUT: &[u8] = b"<?xml version=\"1.0\"?>\n<ConfigDumpInfo><ConfigVersions><Metadata name=\"Catalog.Items\" id=\"new-id\" configVersion=\"new-version\"/></ConfigVersions></ConfigDumpInfo>\n";

struct Fixture {
    workspace: tempfile::TempDir,
    config: PathBuf,
    work: PathBuf,
}

impl Fixture {
    fn new(scenario: &str) -> Self {
        let workspace = temp_workspace();
        let root = workspace.path();
        let config = root.join("v8project.yaml");
        let work = root.join("work");
        let platform = root.join("1cv8");
        for source_set in ["main", "ext"] {
            let source = root.join(source_set);
            fs::create_dir_all(source.join("CommonModules/Example/Ext")).expect("source tree");
            fs::write(source.join("Configuration.xml"), "<Configuration />").expect("root XML");
            fs::write(
                source.join("CommonModules/Example/Ext/Module.bsl"),
                "Procedure Example()\nEndProcedure\n",
            )
            .expect("module");
            fs::write(source.join("ConfigDumpInfo.xml"), BASELINE).expect("baseline CDFI");
        }
        fs::write(root.join("scenario"), scenario).expect("scenario");
        fs::write(root.join("platform-output.xml"), PLATFORM_OUTPUT).expect("platform bytes");
        fs::write(
            &config,
            format!(
                "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/cdfi-recovery-test'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext\n    type: EXTENSION\n    path: ext\ntools:\n  platform:\n    path: '{}'\n",
                work.display(),
                platform.display()
            ),
        )
        .expect("config");
        // The subprocess owns the mutation, including the obstruction of recovery.
        // This guards against implementations that restore only synthetic in-process failures.
        write_shell_script(
            &platform,
            r#"set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
scenario=$(cat "$root/scenario")
source_dir=''
extension=''
out=''
operation=''
previous=''
for arg in "$@"; do
    case "$previous" in
        /LoadConfigFromFiles) source_dir=$arg ;;
        -Extension) extension=$arg ;;
        /Out) out=$arg ;;
    esac
    case "$arg" in
        /LoadConfigFromFiles) operation=load ;;
        /UpdateDBCfg) operation=update ;;
    esac
    previous=$arg
done
printf '%s\n' "$*" >> "$root/calls.log"
if [ -n "$out" ]; then printf 'fake Designer operation %s\n' "$operation" > "$out"; fi
if [ "$operation" = load ]; then
    cp "$root/platform-output.xml" "$source_dir/ConfigDumpInfo.xml"
    if [ "$scenario" = block-recovery ]; then
        rm "$source_dir/ConfigDumpInfo.xml"
        mkdir "$source_dir/ConfigDumpInfo.xml"
        printf 'keep obstruction' > "$source_dir/ConfigDumpInfo.xml/owned-by-platform"
        exit 17
    fi
    if [ "$scenario" = fail-load ]; then exit 17; fi
fi
if [ "$operation" = update ]; then
    if [ "$scenario" = fail-update ]; then exit 17; fi
    if [ "$scenario" = fail-ext-update ] && [ "$extension" = ext ]; then exit 17; fi
fi
exit 0"#,
        );
        Self {
            workspace,
            config,
            work,
        }
    }

    fn cdfi(&self, source_set: &str) -> PathBuf {
        self.workspace
            .path()
            .join(source_set)
            .join("ConfigDumpInfo.xml")
    }

    fn run(&self, flags: &[&str]) -> (std::process::ExitStatus, Value) {
        let output = v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .args(["--json-message", "build", "--full-rebuild"])
            .args(flags)
            .output()
            .expect("run build");
        let payload = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "expected JSON: {error}; stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status, payload)
    }
}

fn assert_platform_failure(status: std::process::ExitStatus, payload: &Value) {
    assert_eq!(status.code(), Some(4), "{payload}");
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    let failed_step = payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["ok"] == false)
        .expect("failed step");
    assert!(failed_step["message"]
        .as_str()
        .expect("message")
        .contains("exit code 17"));
}

fn assert_recovery(payload: &Value, step: usize, action: &str, tracked: &Path) {
    let receipt = &payload["data"]["steps"][step]["cdfi_recovery"];
    assert_eq!(receipt["action"], action, "{payload}");
    let canonical_tracked = fs::canonicalize(tracked.parent().expect("source directory"))
        .expect("canonical source directory")
        .join(tracked.file_name().expect("CDFI filename"));
    assert_eq!(
        receipt["tracked_path"],
        canonical_tracked.to_string_lossy().as_ref()
    );
    assert!(receipt.get("changed_entry_count").is_some(), "{receipt}");
}

#[test]
fn failed_designer_load_restores_exact_bom_crlf_baseline() {
    let fixture = Fixture::new("fail-load");
    assert!(BASELINE.starts_with(b"\xef\xbb\xbf"));
    assert!(BASELINE.windows(2).any(|pair| pair == b"\r\n"));
    let (status, payload) = fixture.run(&["--source-set", "main"]);

    assert_platform_failure(status, &payload);
    assert_eq!(
        fs::read(fixture.cdfi("main")).expect("restored CDFI"),
        BASELINE
    );
    assert_recovery(&payload, 0, "restored", &fixture.cdfi("main"));
    assert_eq!(
        payload["data"]["steps"][0]["cdfi_recovery"]["original_existed"],
        true
    );
    let calls = fs::read_to_string(fixture.workspace.path().join("calls.log")).expect("calls");
    assert!(
        !calls.contains("/UpdateDBCfg"),
        "failed load must stop the pipeline"
    );
}

#[test]
fn failed_update_restores_cdfi_mutated_by_successful_load() {
    let fixture = Fixture::new("fail-update");
    let (status, payload) = fixture.run(&["--source-set", "main"]);

    assert_platform_failure(status, &payload);
    assert_eq!(
        fs::read(fixture.cdfi("main")).expect("restored CDFI"),
        BASELINE
    );
    assert_recovery(&payload, 0, "restored", &fixture.cdfi("main"));
    let calls = fs::read_to_string(fixture.workspace.path().join("calls.log")).expect("calls");
    assert!(calls.contains("/LoadConfigFromFiles") && calls.contains("/UpdateDBCfg"));
}

#[test]
fn failed_load_removes_cdfi_created_without_a_baseline() {
    let fixture = Fixture::new("fail-load");
    fs::remove_file(fixture.cdfi("main")).expect("remove baseline");
    let (status, payload) = fixture.run(&["--source-set", "main"]);

    assert_platform_failure(status, &payload);
    assert!(!fixture.cdfi("main").exists());
    assert_recovery(&payload, 0, "removed_created_file", &fixture.cdfi("main"));
    assert_eq!(
        payload["data"]["steps"][0]["cdfi_recovery"]["original_existed"],
        false
    );
}

#[test]
fn obstructed_recovery_retains_readable_snapshot_and_platform_failure() {
    let fixture = Fixture::new("block-recovery");
    let (status, payload) = fixture.run(&["--source-set", "main"]);

    assert_platform_failure(status, &payload);
    assert_recovery(&payload, 0, "failed", &fixture.cdfi("main"));
    let receipt = &payload["data"]["steps"][0]["cdfi_recovery"];
    let snapshot = receipt["snapshot_path"]
        .as_str()
        .expect("retained snapshot path");
    assert_eq!(
        fs::read(snapshot).expect("read recovery snapshot"),
        BASELINE
    );
    assert!(fixture.cdfi("main").join("owned-by-platform").is_file());
}

#[test]
fn failed_extension_restores_only_its_step_after_main_was_committed() {
    let fixture = Fixture::new("fail-ext-update");
    let (status, payload) = fixture.run(&[]);

    assert_platform_failure(status, &payload);
    assert_eq!(payload["data"]["steps"][0]["source_set"], "main");
    assert_eq!(payload["data"]["steps"][0]["ok"], true);
    assert_eq!(
        fs::read(fixture.cdfi("main")).expect("successful main"),
        PLATFORM_OUTPUT
    );
    assert_eq!(payload["data"]["steps"][1]["source_set"], "ext");
    assert_eq!(
        fs::read(fixture.cdfi("ext")).expect("restored extension"),
        BASELINE
    );
    assert_recovery(&payload, 1, "restored", &fixture.cdfi("ext"));
}

#[test]
fn preview_creates_no_recovery_snapshot_and_never_runs_designer() {
    let fixture = Fixture::new("fail-load");
    let (status, payload) = fixture.run(&["--dry-run"]);

    assert!(status.success(), "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert!(
        !fixture.work.join("temp").exists(),
        "preview must not materialize recovery snapshots"
    );
    for step in payload["data"]["steps"].as_array().expect("preview steps") {
        assert!(step.get("cdfi_recovery").is_none(), "{step}");
    }
    assert!(!fixture.workspace.path().join("calls.log").exists());
    for source_set in ["main", "ext"] {
        assert_eq!(
            fs::read(fixture.cdfi(source_set)).expect("untouched CDFI"),
            BASELINE
        );
    }
}
