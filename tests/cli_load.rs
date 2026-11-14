#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

fn write_designer_script(path: &Path, calls_log: &Path) {
    let body = format!(
        "args=\"$*\"\nout=\"\"\nreport=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  if [ \"$prev\" = \"-ReportFile\" ]; then report=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"{}\"\nif [ -n \"$out\" ]; then mkdir -p \"$(dirname \"$out\")\"; : > \"$out\"; fi\nif printf '%s' \"$args\" | grep -F -q -- '/CompareCfg'; then\n  if printf '%s' \"$args\" | grep -F -q -- 'VendorConfiguration'; then\n    printf 'Configuration Vendor configuration is not available\\n' > \"$out\"\n    exit 17\n  fi\n  if printf '%s' \"$args\" | grep -F -q -- 'ExtensionDBConfiguration'; then\n    if printf '%s' \"$args\" | grep -F -q -- 'ExistingExt'; then\n      : > \"$report\"\n      exit 0\n    fi\n    printf 'extension not found\\n' > \"$out\"\n    exit 19\n  fi\nfi\nexit 0",
        calls_log.display()
    );
    write_script(path, &body);
}

/// A fake `ibcmd` answering the one structural question the load path asks: which extensions the
/// infobase lists. The records go to a file and the script only `cat`s it — shell escaping differs
/// between `bash` and `dash`, and an escaped quote that survives on one and not the other turned a
/// listed extension into an absent one on Linux.
fn write_extension_list_ibcmd(path: &Path, installed: &[&str]) {
    let records = installed
        .iter()
        .map(|name| {
            format!(
                "name                         : \"{name}\"\nversion                      : \nactive                       : yes\npurpose                      : customization\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"{name}-hash\"\n\n"
            )
        })
        .collect::<String>();
    let records_path = path.with_extension("records");
    fs::write(&records_path, records).expect("write records");
    write_script(path, &format!("cat \"{}\"\nexit 0", records_path.display()));
}

fn write_edt_configuration_source(path: &Path, project_name: &str) {
    fs::create_dir_all(path.join("metadata")).expect("metadata");
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{V8_CONFIGURATION_NATURE}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!("Manifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"),
    )
    .expect("manifest");
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration />",
    )
    .expect("descriptor");
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

fn write_config(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    format: &str,
) {
    let config = format!(
        "workPath: '{}'\nformat: {}\ninfobase:\n  connection: 'File=ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        format,
        platform_path.display(),
    );
    fs::write(path, config).expect("config");
}

fn setup_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = base_path.join("v8project.yaml");
    let binary_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");

    fs::create_dir_all(base_path.join("main")).expect("main");
    fs::create_dir_all(&work_path).expect("work");
    write_designer_script(&binary_path, &calls_log);
    write_extension_list_ibcmd(
        &dir.path().join("ibcmd"),
        &["ExistingExt", "UnsupportedExt"],
    );
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &binary_path,
        "DESIGNER",
    );

    (dir, config_path, binary_path, base_path, calls_log)
}

#[test]
fn load_dry_run_plans_without_probing_or_applying() {
    let (_dir, config_path, _binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cf",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(data["provider_dispatched"], false);
    assert_eq!(data["artifact_type"], "configuration_cf");
    // The probe is itself a Designer run, so a preview reports that it was never asked.
    assert_eq!(data["compatibility_state"], "not_probed");
    assert_eq!(data["execution"]["payload"]["applied"], false);
    assert_eq!(data["execution"]["payload"]["update_db_cfg_ran"], false);
    let message = data["message"].as_str().expect("message");
    assert!(message.contains("previewed; nothing applied"), "{message}");
    assert!(!message.contains("applied successfully"), "{message}");
    assert!(
        !calls_log.exists(),
        "preview must not dispatch the platform"
    );
}

#[test]
fn vendor_update_preview_names_masked_argv_without_running_designer() {
    let (dir, config, _binary, base, calls) = setup_project();
    let yaml = fs::read_to_string(&config).expect("read config");
    fs::write(
        &config,
        yaml.replace(
            "  connection: 'File=ib'",
            "  connection: 'File=ib'\n  user: Admin\n  password: preview-secret",
        ),
    )
    .expect("config");
    fs::write(base.join("vendor.cfu"), "update").expect("package");
    fs::write(
        base.join("settings.xml"),
        r#"<Settings xmlns="http://v8.1c.ru/8.3/config/merge/settings" version="1.2"><Objects/></Settings>"#,
    )
    .expect("settings");
    let output = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "upload",
            "vendor.cfu",
            "--mode",
            "update",
            "--settings",
            "settings.xml",
            "--dry-run",
        ])
        .output()
        .expect("upload preview");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(answer["data"]["provider_dispatched"], false);
    let message = answer["data"]["message"].as_str().expect("message");
    for part in ["/UpdateCfg", "vendor.cfu", "-Settings", "settings.xml"] {
        assert!(message.contains(part), "{message}");
    }
    let private_copy_root = dir.path().join("work").join("temp");
    assert!(
        message.contains(&private_copy_root.display().to_string()),
        "{message}"
    );
    assert!(!message.contains("preview-secret"), "{message}");
    assert!(!calls.exists());
    assert_eq!(
        fs::read_dir(dir.path().join("work")).expect("work").count(),
        0
    );
}

#[test]
fn vendor_update_requires_a_version_change_before_updating_the_database() {
    for (before, after, expected_steps, succeeds) in [
        ("1.0.0.1", "1.0.0.2", "read\nupdate\nread\napply\n", true),
        ("1.0.0.1", "1.0.0.1", "read\nupdate\nread\n", false),
        ("1.0.0.1", "", "read\nupdate\nread\n", false),
        ("", "1.0.0.2", "read\n", false),
    ] {
        let (_dir, config, binary, base, calls) = setup_project();
        let update_marker = calls.with_extension("updated");
        write_script(
            &binary,
            &format!(
                r#"previous=''
target=''
list=''
package=''
settings=''
step=''
for arg do
    case "$previous" in
        /DumpConfigToFiles) target="$arg" ;;
        -listFile) list="$arg" ;;
        /UpdateCfg) package="$arg" ;;
        -Settings) settings="$arg" ;;
    esac
    case "$arg" in
        /DumpConfigToFiles) step=read ;;
        /UpdateCfg) step=update ;;
        /UpdateDBCfg) step=apply ;;
        /CompareCfg|/LoadCfg|/MergeCfg) exit 51 ;;
    esac
    previous="$arg"
done
printf '%s\n' "$step" >> "{calls}"
case "$step" in
    read)
        test "$(cat "$list")" = Configuration || exit 52
        version='{before}'
        printf 'this file changed after validation
' > "{base}/merge settings.xml"
        if test -f "{marker}"; then version='{after}'; fi
        if test -n "$version"; then
            printf '<MetaDataObject version="2.20"><Configuration><Properties><Version>%s</Version></Properties></Configuration></MetaDataObject>\n' "$version" > "$target/Configuration.xml"
        fi
        ;;
    update)
        test "$package" = "{base}/vendor update.cfu" || exit 53
        test "$settings" != "{base}/merge settings.xml" || exit 54
        grep -Fq '<Objects/>' "$settings" || exit 55
        : > "{marker}"
        ;;
esac
"#,
                calls = calls.display(),
                marker = update_marker.display(),
                base = base
                    .canonicalize()
                    .expect("canonical project path")
                    .display(),
            ),
        );
        fs::write(base.join("vendor update.cfu"), "update").expect("package");
        fs::write(
            base.join("merge settings.xml"),
            r#"<Settings xmlns="http://v8.1c.ru/8.3/config/merge/settings" version="1.2"><Objects/></Settings>"#,
        )
        .expect("settings");
        let output = v8_runner_command()
            .args([
                "--config",
                &config.display().to_string(),
                "--json-message",
                "upload",
                "vendor update.cfu",
                "--mode",
                "update",
                "--settings",
                "merge settings.xml",
            ])
            .output()
            .expect("upload");
        let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(output.status.success(), succeeds, "{answer}");
        assert_eq!(answer["data"]["provider_dispatched"], true, "{answer}");
        assert_eq!(answer["data"]["mode"], "update", "{answer}");
        assert_eq!(
            answer["data"]["artifact_type"], "vendor_update_cfu",
            "{answer}"
        );
        let result = &answer["data"]["execution"]["payload"];
        assert_eq!(result["applied"], succeeds, "{answer}");
        assert_eq!(result["update_db_cfg_ran"], succeeds, "{answer}");
        assert_eq!(fs::read_to_string(calls).expect("calls"), expected_steps);
    }
}

#[test]
fn load_cf_json_success_loads_and_updates_without_asking() {
    let (_dir, config_path, _binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cf",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "upload");
    assert_eq!(payload["data"]["artifact_type"], "configuration_cf");
    assert_eq!(payload["data"]["compatibility_state"], "not_probed");
    assert_eq!(payload["data"]["execution"]["payload"]["applied"], true);
    assert_eq!(
        payload["data"]["execution"]["payload"]["update_db_cfg_ran"],
        true
    );
    assert!(payload["data"]["platform_log_path"].is_string());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(
        !calls.contains("/CompareCfg"),
        "a configuration load states its mode and asks nothing: {calls}"
    );
    assert!(calls.contains("/LoadCfg"));
    assert!(calls.contains("/UpdateDBCfg"));
}

#[test]
fn upload_update_failure_preserves_the_completed_load_receipt() {
    let (_dir, config_path, binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");

    let script = fs::read_to_string(&binary_path).expect("designer script");
    let final_exit = script.rfind("\nexit 0").expect("final success exit");
    fs::write(
        &binary_path,
        format!(
            "{}\nif printf '%s' \"$args\" | grep -F -q -- '/UpdateDBCfg'; then exit 23; fi\nexit 0\n",
            &script[..final_exit]
        ),
    )
    .expect("failing designer script");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "upload",
            "--path",
            "release.cf",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "upload");
    let data = &payload["data"];
    assert_eq!(data["provider"]["selected"], "designer");
    assert_eq!(data["provider_dispatched"], true);
    assert_eq!(data["execution"]["status"], "failed");
    assert_eq!(data["execution"]["payload"]["applied"], true);
    assert_eq!(data["execution"]["payload"]["update_db_cfg_ran"], true);

    let calls = fs::read_to_string(calls_log).expect("calls");
    let load = calls.find("/LoadCfg").expect("load ran");
    let update = calls.find("/UpdateDBCfg").expect("update ran");
    assert!(load < update, "load must precede failed update: {calls}");
}

#[test]
fn upload_rejects_a_cfe_name_mismatch_before_loading_the_target() {
    for (reference, installed, compatibility) in [
        ("DeliberatelyOtherName", &[][..], "absent"),
        ("DeliberatelyOtherName", &["YAXUNIT"][..], "absent"),
        (
            "DeliberatelyOtherName",
            &["DeliberatelyOtherName"][..],
            "supported",
        ),
        ("YAxUnit", &[][..], "absent"),
    ] {
        let (dir, config_path, binary, base_path, calls_log) = setup_project();
        fs::write(base_path.join("release.cfe"), "YAXUNIT").expect("package");
        write_extension_list_ibcmd(&dir.path().join("ibcmd"), installed);
        write_cfe_probe_designer(&binary, &calls_log, "YAXUNIT");

        let output = v8_runner_command()
            .args(["--config"])
            .arg(&config_path)
            .args([
                "--json-message",
                "upload",
                "release.cfe",
                "--ref",
                reference,
            ])
            .output()
            .expect("upload");
        let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert!(!output.status.success(), "{answer}");
        let data = &answer["data"];
        assert_eq!(data["compatibility_state"], compatibility);
        assert_eq!(data["execution"]["payload"]["applied"], false);
        assert_eq!(data["execution"]["payload"]["update_db_cfg_ran"], false);
        let message = data["message"].as_str().expect("diagnostic");
        assert!(
            message.contains(reference) && message.contains("YAXUNIT"),
            "{message}"
        );
        let calls = fs::read_to_string(calls_log).expect("Designer calls");
        assert!(calls.contains("/DumpConfigToFiles"), "{calls}");
        assert_eq!(calls.matches("/LoadCfg").count(), 1, "{calls}");
        assert!(calls.contains("-Extension UploadPackageProbe"), "{calls}");
        assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
        assert!(
            !calls.contains(&format!("File={}", base_path.join("ib").display())),
            "{calls}"
        );
    }
}

fn write_cfe_probe_designer(path: &Path, calls_log: &Path, internal_name: &str) {
    let body = format!(
        r#"printf '%s\n' "$*" >> "{}"
prev=""
for arg in "$@"; do
  case "$prev" in
    /Out) : > "$arg" ;;
    /DumpConfigToFiles)
      mkdir -p "$arg"
      printf '%s\n' '<MetaDataObject><Configuration><Properties><Name>{internal_name}</Name></Properties></Configuration></MetaDataObject>' > "$arg/Configuration.xml"
      ;;
  esac
  prev="$arg"
done
exit 0"#,
        calls_log.display()
    );
    write_script(path, &body);
}

#[test]
fn upload_refuses_a_cfe_when_designer_did_not_export_its_descriptor() {
    let (_dir, config_path, _binary, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cfe"), "cfe").expect("package");

    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "upload",
            "release.cfe",
            "--ref",
            "FirstExt",
        ])
        .output()
        .expect("upload");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(!output.status.success(), "{answer}");
    let data = &answer["data"];
    assert_eq!(data["execution"]["payload"]["applied"], false);
    assert!(data["message"]
        .as_str()
        .expect("diagnostic")
        .contains("cannot establish the CFE name before upload"));
    let calls = fs::read_to_string(calls_log).expect("Designer calls");
    assert!(calls.contains("/DumpConfigToFiles"), "{calls}");
    assert_eq!(calls.matches("/LoadCfg").count(), 1, "{calls}");
    assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
}

#[test]
fn first_extension_load_json_reports_absent_and_applies_in_order() {
    let (dir, config_path, binary_path, base_path, calls_log) = setup_project();
    let work_path = dir.path().join("work;part=release");
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &binary_path,
        "DESIGNER",
    );
    fs::write(base_path.join("release.cfe"), "cfe").expect("artifact");
    write_cfe_probe_designer(&binary_path, &calls_log, "FirstExt");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cfe",
            "--ref",
            "FirstExt",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["extension"], "FirstExt");
    assert_eq!(payload["data"]["compatibility_state"], "absent");
    assert_eq!(
        payload["data"]["execution"]["payload"]["compatibility_state"],
        "absent"
    );
    let calls = fs::read_to_string(calls_log).expect("calls");
    let scratch = format!("{}/temp/upload-cfe-", work_path.display());
    assert!(
        calls.contains(&format!("CREATEINFOBASE File='{scratch}")),
        "{calls}"
    );
    assert!(calls.contains(&format!("/F {scratch}")), "{calls}");
    assert!(
        !calls.contains("/CompareCfg"),
        "absence comes from the infobase's own list, so nothing is compared: {calls}"
    );
    let ordered = [
        calls
            .find("/DumpConfigToFiles")
            .expect("scratch descriptor"),
        calls.rfind("/LoadCfg").expect("target load"),
        calls.find("/UpdateDBCfg").expect("target update"),
    ];
    assert!(
        ordered[0] < ordered[1] && ordered[1] < ordered[2],
        "{calls}"
    );
    let packages: Vec<_> = calls
        .lines()
        .filter_map(|line| line.split_once("/LoadCfg "))
        .map(|(_, rest)| rest.split_once(" -Extension ").expect("extension flag").0)
        .collect();
    assert_eq!(packages.len(), 2, "{calls}");
    assert_eq!(packages[0], packages[1], "{calls}");
    assert!(!Path::new(packages[0]).exists(), "private copy was removed");
}

#[test]
fn load_text_success_is_compact_and_keeps_target_visible() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    let artifact_path = base_path.join("release.cf");
    fs::write(&artifact_path, "cf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "load",
            "--path",
            "release.cf",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("● Artifact load completed successfully"));
    assert!(stdout.contains("│   target: configuration"));
    assert!(stdout.contains("│   action: load cf"));
    assert!(stdout.contains(artifact_path.display().to_string().as_str()));
    assert!(!stdout.contains("platform log"));
}

#[test]
fn merge_cfe_json_success_requires_extension_and_settings() {
    let (_dir, config_path, _binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cfe"), "cfe").expect("artifact");
    fs::write(base_path.join("merge.xml"), "<settings/>").expect("settings");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cfe",
            "--mode",
            "merge",
            "--settings",
            "merge.xml",
            "--extension",
            "ExistingExt",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["artifact_type"], "extension_cfe");
    assert_eq!(payload["data"]["extension"], "ExistingExt");
    assert_eq!(payload["data"]["compatibility_state"], "supported");
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(
        !calls.contains("/CompareCfg"),
        "an extension is answered by the infobase list: {calls}"
    );
    assert!(calls.contains("/MergeCfg"));
    assert!(calls.contains("-Settings"));
    assert!(calls.contains("-Extension ExistingExt"));
    assert!(calls.contains("/UpdateDBCfg -Extension ExistingExt"));
}

#[test]
fn upload_ref_names_an_extension_set_by_the_set_name_like_push() {
    // The set `sales` holds sources whose `Configuration.xml` names another extension;
    // `--ref sales` still targets `sales`, as `push` does. A name that matches no set goes
    // to the platform as given.
    let descriptor = "<MetaDataObject><Configuration><Properties><Name>OtherExt</Name><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>";
    for (flag, reference, descriptor, target) in [
        ("--ref", "sales", Some(descriptor), "sales"),
        ("--extension", "sales", Some(descriptor), "sales"),
        (
            "--ref",
            "sales",
            Some("<MetaDataObject><Configuration>"),
            "sales",
        ),
        ("--ref", "sales", None, "sales"),
        ("--ref", "ExistingExt", Some(descriptor), "ExistingExt"),
    ] {
        let (dir, config_path, _binary_path, base_path, calls_log) = setup_project();
        let project = fs::read_to_string(&config_path).expect("read project");
        fs::write(
            &config_path,
            project.replacen(
                "
tools:",
                "
  - name: sales
    type: EXTENSION
    path: addon
tools:",
                1,
            ),
        )
        .expect("declare extension source set");
        let addon = base_path.join("addon");
        fs::create_dir(&addon).expect("extension source directory");
        if let Some(xml) = descriptor {
            fs::write(addon.join("Configuration.xml"), xml).expect("extension descriptor");
        }
        write_extension_list_ibcmd(
            &dir.path().join("ibcmd"),
            &["ExistingExt", "OtherExt", "sales"],
        );
        fs::write(base_path.join("release.cfe"), "cfe").expect("package");
        fs::write(base_path.join("settings.xml"), "<Settings/>").expect("settings");

        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "upload",
                "release.cfe",
                "--mode",
                "combine",
                "--settings",
                "settings.xml",
                flag,
                reference,
            ])
            .output()
            .expect("upload");
        let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert!(output.status.success(), "{flag} {reference}: {answer}");
        assert_eq!(answer["data"]["extension"], target, "{answer}");
        assert_eq!(
            answer["data"]["compatibility_state"], "supported",
            "{answer}"
        );
        let calls = fs::read_to_string(&calls_log).expect("Designer calls");
        assert!(
            calls.lines().any(|line| line.contains("/MergeCfg")
                && line.ends_with(&format!("-Extension {target}"))),
            "{calls}"
        );
        assert!(
            calls.contains(&format!("/UpdateDBCfg -Extension {target}")),
            "{calls}"
        );
        assert!(!calls.contains("OtherExt"), "{calls}");
    }
}

#[test]
fn upload_update_without_settings_returns_validation_payload() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cf",
            "--mode",
            "update",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "upload");
    assert_eq!(payload["data"]["mode"], "update");
    assert_eq!(payload["data"]["execution"]["payload"]["applied"], false);
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("requires --settings"));
}

#[test]
fn load_text_failure_surfaces_structured_error() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "load",
            "--path",
            "release.cf",
            "--mode",
            "update",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✖ Artifact load failed"));
    assert!(stdout.contains("[error:artifact_load_failed]"));
    assert!(stdout.contains("requires --settings"));
}

#[test]
fn load_rejects_edt_format_even_with_designer_builder() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    fs::write(base_path.join("release.cf"), "cf").expect("artifact");
    write_edt_configuration_source(&base_path.join("main"), "main");
    write_config(
        &config_path,
        &base_path,
        &config_path.parent().expect("parent").join("work"),
        &config_path.parent().expect("parent").join("1cv8"),
        "EDT",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.cf",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("the Designer provider and format=DESIGNER"));
}

#[test]
fn load_rejects_unknown_artifact_type_with_unknown_payload_metadata() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    fs::write(base_path.join("release.zip"), "zip").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "release.zip",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["artifact_type"], "unknown");
    assert_eq!(payload["data"]["target_kind"], "unknown");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("only .cf and .cfe"));
}

#[test]
fn load_rejects_external_artifact_type_with_unknown_target_kind_payload_metadata() {
    let (_dir, config_path, _binary_path, base_path, _calls_log) = setup_project();
    fs::write(base_path.join("tool.epf"), "epf").expect("artifact");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "load",
            "--path",
            "tool.epf",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(
        payload["data"]["artifact_type"],
        "external_data_processor_epf"
    );
    assert_eq!(payload["data"]["target_kind"], "unknown");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("only .cf and .cfe"));
}

/// Режим назван именем словаря: `upload --mode combine` доходит до платформы, ответ
/// называет режим новым именем, а прежнее `merge` принимается ещё один цикл выпуска.
#[test]
fn upload_mode_combine_reaches_the_platform_and_answers_under_the_new_name() {
    for mode in ["combine", "merge"] {
        let (_dir, config_path, _binary_path, base_path, calls_log) = setup_project();
        fs::write(base_path.join("release.cfe"), "cfe").expect("artifact");
        fs::write(base_path.join("merge.xml"), "<settings/>").expect("settings");

        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "upload",
                "--path",
                "release.cfe",
                "--mode",
                mode,
                "--settings",
                "merge.xml",
                "--extension",
                "ExistingExt",
            ])
            .output()
            .expect("run command");

        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(payload["ok"], true, "{mode}: {payload}");
        assert_eq!(payload["command"], "upload", "{mode}: {payload}");
        assert_eq!(payload["data"]["mode"], "combine", "{mode}: {payload}");
        let calls = fs::read_to_string(&calls_log).expect("calls");
        assert!(calls.contains("/MergeCfg"), "{mode}: {calls}");
    }
}

/// #4: `upload` при `providers.push: ibcmd` и конфиге, открытом из другого каталога, отдаёт
/// `ibcmd` и Конфигуратору пути, посчитанные от каталога `v8project.yaml`, без `.` внутри.
#[test]
fn upload_with_ibcmd_push_receives_config_relative_paths_resolved_from_the_config_directory() {
    let dir = temp_workspace();
    let project = dir.path().join("project");
    let elsewhere = dir.path().join("elsewhere");
    let calls_log = dir.path().join("calls.log");
    let platform = project.join("platform");
    fs::create_dir_all(project.join("main")).expect("main");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    fs::write(project.join("release.cfe"), "cfe").expect("artifact");
    write_cfe_probe_designer(&platform.join("1cv8"), &calls_log, "FirstExt");
    let records = platform.join("ibcmd.records");
    fs::write(&records, "").expect("records");
    write_script(
        &platform.join("ibcmd"),
        &format!(
            "printf 'ibcmd %s\\n' \"$*\" >> \"{}\"\ncat \"{}\"\nexit 0",
            calls_log.display(),
            records.display()
        ),
    );
    fs::write(
        project.join("v8project.yaml"),
        "workPath: ./work\nformat: DESIGNER\nproviders:\n  push: ibcmd\ninfobase:\n  connection: 'File=./ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: ./main\ntools:\n  platform:\n    path: ./platform/1cv8\n",
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            "../project/v8project.yaml",
            "--json-message",
            "upload",
            "--path",
            "./release.cfe",
            "--extension",
            "FirstExt",
        ])
        .current_dir(&elsewhere)
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project = fs::canonicalize(&project).expect("canonical project");
    let infobase = project.join("ib");
    let calls = fs::read_to_string(&calls_log).expect("calls");
    assert!(
        calls.contains(&format!(
            "ibcmd infobase --data {} --db-path {} ",
            project.join("work").join("ibcmd-data").display(),
            infobase.display()
        )),
        "{calls}"
    );
    let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        answer["data"]["artifact_path"],
        project.join("release.cfe").display().to_string()
    );
    assert!(
        calls.contains(&format!("File={}", infobase.display())),
        "{calls}"
    );
    assert!(!calls.contains("/./"), "{calls}");
}

/// Образ базы — не пакет: `upload ib.dt` отказывает до платформы и называет
/// `infobase restore`. Отказ не зависит от того, есть ли такой файл.
#[test]
fn upload_of_a_transfer_file_names_infobase_restore() {
    let (_dir, config_path, _binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("ib.dt"), "dt").expect("transfer file");

    for file in ["ib.dt", "missing.DT"] {
        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "upload",
                file,
            ])
            .output()
            .expect("run command");

        assert_eq!(output.status.code(), Some(2), "{file}");
        let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(payload["command"], "upload");
        assert_eq!(payload["error"]["kind"], "validation", "{payload}");
        assert!(
            payload["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("`infobase restore`")),
            "{file}: {payload}"
        );
        assert!(
            !calls_log.exists(),
            "{file}: the platform must not be started"
        );
    }
}

/// Секция `infobase.dbms` — доступ к СУБД. У кластера в СУБД раннер перед `upload .cfe` не
/// ходит: список расширений спрашивает агент (#431), поэтому без секции загрузка не
/// отказывает из-за неё и `ibcmd` не запускает. Агента в тесте нет — команда отказывает
/// по нему, а не по секции.
#[test]
fn a_cluster_extension_upload_needs_no_dbms_section() {
    let (dir, config_path, _binary_path, base_path, calls_log) = setup_project();
    fs::write(base_path.join("release.cfe"), "cfe").expect("artifact");
    let ibcmd_started = dir.path().join("ibcmd-started");
    write_script(
        &dir.path().join("ibcmd"),
        &format!("touch '{}'\nexit 0", ibcmd_started.display()),
    );
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace(
            "connection: 'File=ib'",
            "connection: 'Srvr=127.0.0.1:1541;Ref=demo'",
        ) + "  designer_agent:\n    startup_timeout_ms: 200\n    live: false\n",
    )
    .expect("server config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "upload",
            "--path",
            &base_path.join("release.cfe").display().to_string(),
            "--extension",
            "ExistingExt",
        ])
        .output()
        .expect("run command");

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(!output.status.success(), "{payload}");
    assert_eq!(payload["error"]["kind"], "environment", "{payload}");
    assert!(
        !payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("infobase.dbms"),
        "{payload}"
    );
    assert!(!ibcmd_started.exists(), "ibcmd must not be started");
    let calls = fs::read_to_string(&calls_log).unwrap_or_default();
    assert!(!calls.contains("/LoadCfg"), "nothing is loaded: {calls}");
}
