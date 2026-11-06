mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use support::command_data::assert_data_matches_its_command_form;

const VERSION_FILE: &str = "ConfigDumpInfo.xml";
const BASELINE: &str = "<ConfigDumpInfo version=\"2.0\"/>";

fn platform(parent: &Path) -> PathBuf {
    let dir = parent.join("platform");
    let designer = dir.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
    if !designer.exists() {
        fs::create_dir_all(&dir).unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_diff_platform.rs");
        let compile = Command::new("rustc")
            .args(["--edition=2021"])
            .arg(source)
            .arg("-o")
            .arg(&designer)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        fs::copy(
            designer,
            dir.join(format!("ibcmd{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
    }
    dir
}

struct Project {
    root: PathBuf,
    base: PathBuf,
}

impl Project {
    fn new(parent: &Path, name: &str, provider: Option<&str>) -> Self {
        let root = parent.join(name);
        let base = parent.join("bases/ib");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("1Cv8.1CD"), b"fake database").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join(VERSION_FILE), BASELINE).unwrap();
        let providers = provider.map_or(String::new(), |value| {
            format!("providers:\n  diff: {value}\n")
        });
        fs::write(root.join("v8project.yaml"), format!(
            "workPath: 'рабочая папка'\nformat: DESIGNER\n{providers}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: '{}'\n",
            platform(parent).display(),
        )).unwrap();
        fs::write(
            root.join("v8project.local.yaml"),
            format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n",
                base.display(),
            ),
        )
        .unwrap();
        Self { root, base }
    }

    fn run(&self, args: &[&str], output: &str) -> Output {
        let argv = self.root.join("argv.txt");
        if argv.exists() {
            fs::remove_file(&argv).unwrap();
        }
        support::v8_runner_command()
            .current_dir(&self.root)
            .arg("--json-message")
            .arg("diff")
            .args(args)
            .env("DIFF_ARGV", argv)
            .env("DIFF_BASELINE", self.root.join("baseline.txt"))
            .env("DIFF_OUTPUT", output)
            .output()
            .unwrap()
    }

    fn argv(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("argv.txt"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn marker(&self) -> PathBuf {
        self.base
            .parent()
            .unwrap()
            .join(".ib.v8-runner.owners.json")
    }
}

fn envelope(output: Output, ok: bool) -> Value {
    assert_eq!(
        output.status.success(),
        ok,
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).expect("JSON envelope");
    assert_eq!(response["command"], "diff");
    response
}

#[test]
fn diff_lists_metadata_versions_without_dumping() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", None);
    let response = envelope(
        project.run(&[], "added: Catalog.X\\nmodified: CommonModule.Y.Module\\n"),
        true,
    );
    assert_data_matches_its_command_form(&response, "ibcmd object list");
    let data = &response["data"];
    assert_eq!(data["provider"]["selected"], "ibcmd");
    assert_eq!(data["full_dump"], false);
    assert_eq!(
        data["changes"],
        json!([
            {"object":"Catalog.X", "kind":"added"},
            {"object":"CommonModule.Y.Module", "kind":"modified"}
        ])
    );
    let argv = project.argv();
    assert!(argv.windows(2).any(|pair| pair == ["export", "status"]));
    assert!(argv
        .iter()
        .any(|arg| arg.starts_with("--base=") && arg.ends_with(VERSION_FILE)));
    assert_eq!(
        fs::read_to_string(project.root.join("src").join(VERSION_FILE)).unwrap(),
        BASELINE
    );
    assert_eq!(fs::read_dir(project.root.join("src")).unwrap().count(), 1);
}

#[test]
fn designer_lists_named_objects_and_refuses_unrecognized_entries() {
    let dir = tempfile::tempdir().unwrap();
    let designer = Project::new(dir.path(), "designer", Some("designer"));
    let response = envelope(
        designer.run(
            &[],
            "New: Catalog.Каталог\\nModified: CommonModule.Модуль.Module",
        ),
        true,
    );
    assert_eq!(
        response["data"]["changes"],
        json!([
            {"object":"Catalog.Каталог", "kind":"added"},
            {"object":"CommonModule.Модуль.Module", "kind":"modified"}
        ])
    );
    let ibcmd = Project::new(dir.path(), "ibcmd", Some("ibcmd"));
    for (project, bad_output) in [(&designer, "New:  "), (&ibcmd, "modified:  ")] {
        let response = envelope(project.run(&[], bad_output), false);
        assert_eq!(response["error"]["kind"], "invalid_output");
    }
}

#[test]
fn full_dump_and_empty_list_are_distinct_for_both_providers() {
    let dir = tempfile::tempdir().unwrap();
    for (provider, full) in [("ibcmd", "modified: all\\n"), ("designer", "FullDump\\n")] {
        let project = Project::new(dir.path(), provider, Some(provider));
        for (output, is_full) in [(full, true), ("", false)] {
            let response = envelope(project.run(&[], output), true);
            assert_data_matches_its_command_form(&response, provider);
            assert_eq!(response["data"]["full_dump"], is_full);
            assert_eq!(response["data"]["changes"], json!([]));
            if provider == "designer" {
                assert!(project.argv().iter().any(|arg| arg == "-getChanges"));
            }
        }
    }
}

#[test]
fn against_keeps_opaque_report_and_uses_typed_designer_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", None);
    let file = "Пакет поставки.cf";
    fs::write(project.root.join(file), b"package").unwrap();
    for (target, expected_type, parameter) in [
        ("db", "DBConfiguration", None),
        (
            "vendor:Имя поставщика",
            "VendorConfiguration",
            Some(("-SecondName", "Имя поставщика")),
        ),
        (file, "File", Some(("-SecondFile", file))),
    ] {
        let response = envelope(project.run(&["--against", target], "report"), true);
        assert_data_matches_its_command_form(&response, target);
        assert_eq!(response["data"]["provider"]["selected"], "designer");
        assert_eq!(
            response["data"]["provider"]["skipped"][0]["provider"],
            "ibcmd"
        );
        assert!(response["data"]["provider"]["skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("requires designer"));
        let report = Path::new(response["data"]["report"].as_str().unwrap());
        assert_eq!(fs::read(report).unwrap(), b"\xff\xfeR\0\r\0\n\0");
        let argv = project.argv();
        for (key, value) in [
            ("-FirstConfigurationType", "MainConfiguration"),
            ("-SecondConfigurationType", expected_type),
            ("-ReportType", "Full"),
            ("-ReportFormat", "txt"),
        ] {
            assert!(argv.windows(2).any(|pair| pair == [key, value]), "{argv:?}");
        }
        for flag in [
            "/CompareCfg",
            "-IncludeChangedObjects",
            "-IncludeAddedObjects",
            "-IncludeDeletedObjects",
        ] {
            assert!(argv.iter().any(|arg| arg == flag), "{argv:?}");
        }
        if let Some((key, value)) = parameter {
            let actual = argv.windows(2).find(|pair| pair[0] == key).unwrap();
            assert!(actual[1].ends_with(value));
            assert!(!actual[1].starts_with(r"\\?\"));
        }
        let report_arg = &argv
            .windows(2)
            .find(|pair| pair[0] == "-ReportFile")
            .unwrap()[1];
        assert!(report_arg.contains("рабочая папка"));
        assert!(!report_arg.starts_with(r"\\?\"));
    }
    assert!(
        !project.marker().exists(),
        "comparison is read-only at the owner boundary"
    );
}

#[test]
fn designer_output_must_exist_and_partial_reports_are_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", Some("designer"));
    for args in [&[][..], &["--against", "db"][..]] {
        for output in ["missing", "empty", "partial"] {
            let response = envelope(project.run(args, output), false);
            assert_data_matches_its_command_form(&response, output);
            assert_eq!(response["data"]["provider_dispatched"], true);
            assert!(response["data"]["report"].is_null());
            assert_eq!(
                response["error"]["kind"],
                if output == "partial" {
                    "platform"
                } else {
                    "invalid_output"
                }
            );
            let argv = project.argv();
            let path = &argv
                .windows(2)
                .find(|pair| pair[0] == "-ReportFile" || pair[0] == "-getChanges")
                .unwrap()[1];
            assert!(!Path::new(path).exists(), "failed output retained: {path}");
        }
    }
    let response = envelope(project.run(&[], "blank"), false);
    assert_eq!(response["error"]["kind"], "invalid_output");
}

#[test]
fn platform_failure_includes_stdout_without_the_infobase_password() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", Some("designer"));
    let password = "synthetic-diff-password";
    let mut local = fs::OpenOptions::new()
        .append(true)
        .open(project.root.join("v8project.local.yaml"))
        .unwrap();
    write!(local, "    user: operator\n    password: {password}\n").unwrap();
    drop(local);
    let output = project.run(&["--against", "db"], "echo_password");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(password));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(password));
    let response = envelope(output, false);
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("stdout: platform refused"), "{message}");
}

#[test]
fn against_rejects_an_explicit_ibcmd_override_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", Some("ibcmd"));
    let response = envelope(project.run(&["--against", "db"], "report"), false);
    assert_data_matches_its_command_form(&response, "unsupported override");
    assert_eq!(response["error"]["kind"], "capability");
    assert_eq!(response["data"]["provider"]["origin"]["kind"], "override");
    assert_eq!(
        response["data"]["provider"]["skipped"][0]["provider"],
        "ibcmd"
    );
    assert_eq!(response["data"]["provider_dispatched"], false);
    let message = response["error"]["message"].as_str().unwrap();
    for detail in [
        "providers.diff",
        "v8project.yaml",
        "remove the key or assign designer",
    ] {
        assert!(message.contains(detail), "{message}");
    }
    assert!(!project.root.join("argv.txt").exists());
}

#[test]
fn owner_boundary_warns_for_status_and_preserves_marker_for_both_forms() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Project::new(dir.path(), "owner", Some("designer"));
    envelope(owner.run(&[], ""), true);
    let marker = fs::read(owner.marker()).unwrap();
    let other = Project::new(dir.path(), "other", Some("designer"));
    let plain = envelope(other.run(&[], ""), true);
    assert!(plain["warnings"]
        .to_string()
        .contains("of another working copy"));
    let against = envelope(other.run(&["--against", "db"], "report"), true);
    assert!(!against["warnings"]
        .to_string()
        .contains("of another working copy"));
    assert_eq!(fs::read(other.marker()).unwrap(), marker);
}

#[test]
fn version_file_is_resolved_without_rewriting_the_sources() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path(), "project", Some("designer"));
    let work = project.root.join("рабочая папка");
    support::memory::remember_base(
        &project.root.join("рабочая папка"),
        "origin",
        support::memory::Base::File(&project.base),
        &[support::memory::Set::configuration(
            "main",
            &project.root.join("src"),
        )],
    );
    let memory: Value =
        serde_json::from_slice(&fs::read(work.join("infobases/origin/generation.json")).unwrap())
            .unwrap();
    let copy = work.join("infobases/origin/dump-info/main");
    fs::create_dir_all(&copy).unwrap();
    fs::write(copy.join(VERSION_FILE), "saved version file").unwrap();
    fs::write(
        copy.join("identity"),
        memory["main"]["identity"].as_str().unwrap(),
    )
    .unwrap();
    envelope(project.run(&[], ""), true);
    assert_eq!(
        fs::read_to_string(project.root.join("baseline.txt")).unwrap(),
        "saved version file"
    );
    let local = project.root.join("src").join(VERSION_FILE);
    assert_eq!(fs::read_to_string(&local).unwrap(), BASELINE);
    fs::write(copy.join("identity"), "foreign pair").unwrap();
    envelope(project.run(&[], ""), true);
    assert_eq!(
        fs::read_to_string(project.root.join("baseline.txt")).unwrap(),
        BASELINE
    );
    fs::remove_file(local).unwrap();
    let response = envelope(project.run(&[], ""), false);
    assert!(response["error"].to_string().contains(VERSION_FILE));
    assert!(!project.root.join("argv.txt").exists());
}
