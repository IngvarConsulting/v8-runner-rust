mod support;

use support::{temp_workspace, v8_runner_command};

#[test]
fn a_file_base_refuses_sessions_with_the_apply_recipe() {
    let project = temp_workspace();
    let config = project.path().join("v8project.yaml");
    std::fs::write(&config, "format: DESIGNER\n").unwrap();
    std::fs::write(
        project.path().join("v8project.local.yaml"),
        "infobases:\n  origin:\n    connection: 'File=./base'\n",
    )
    .unwrap();
    let output = v8_runner_command()
        .arg("--config")
        .arg(config)
        .args(["--json-message", "sessions", "list"])
        .output()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    assert!(!output.status.success(), "{body}");
    assert_eq!(body["command"], "sessions list", "{body}");
    assert_eq!(body["error"]["code"], "target", "{body}");
    assert!(
        body.to_string().contains("apply --sessions force"),
        "{body}"
    );
}

#[cfg(unix)]
mod rac {
    use std::path::{Path, PathBuf};
    use std::process::Output;

    use super::*;
    use support::write_shell_script;

    const CLIENT: &str = "11111111-1111-1111-1111-111111111111";
    const JOB: &str = "22222222-2222-2222-2222-222222222222";

    struct Project {
        _root: tempfile::TempDir,
        config: PathBuf,
        calls: PathBuf,
    }

    fn project(actions: &str, credentials: &str) -> Project {
        let root = temp_workspace();
        let config = root.path().join("v8project.yaml");
        let calls = root.path().join("calls");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        write_shell_script(
            &bin.join("rac"),
            &format!(
                r#"printf '%s\n' "$*" >> '{}'
case "$1 $2" in
  'cluster list') printf 'cluster : aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\nhost : localhost\nport : 1541\n';;
  'infobase summary') printf 'infobase : bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\nname : fixture\n';;
  {}
esac"#,
                calls.display(),
                actions,
            ),
        );
        std::fs::write(
            &config,
            format!(
                "format: DESIGNER\ntools:\n  platform:\n    path: '{}'\n",
                bin.display()
            ),
        )
        .unwrap();
        std::fs::write(root.path().join("v8project.local.yaml"), format!(
            "infobases:\n  origin:\n    connection: 'Srvr=localhost;Ref=fixture'\n{credentials}    cluster:\n      ras: localhost:1545\n"
        )).unwrap();
        Project {
            _root: root,
            config,
            calls,
        }
    }

    fn run(config: &Path, arguments: &[&str]) -> (Output, serde_json::Value) {
        let output = v8_runner_command()
            .arg("--config")
            .arg(config)
            .arg("--json-message")
            .args(arguments)
            .output()
            .unwrap();
        let body = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output, body)
    }

    fn session(id: &str, number: u32, app: &str) -> String {
        format!("session : {id}\nsession-id : {number}\napp-id : {app}\nhost : localhost\nuser-name : operator\nstarted-at : 2026-11-16T08:00:00\nhibernate : no\n\n")
    }

    #[test]
    fn terminate_all_includes_application_jobs_and_leaves_protected_sessions() {
        let listing = [
            session(CLIENT, 8, "1CV8C"),
            session(JOB, 9, "BackgroundJob"),
            session(
                "33333333-3333-3333-3333-333333333333",
                10,
                "SystemBackgroundJob",
            ),
            session("44444444-4444-4444-4444-444444444444", 11, "RAS"),
            session("55555555-5555-5555-5555-555555555555", 12, "Designer"),
        ]
        .concat();
        let project = project(
            &format!("'session list') printf '%s' '{listing}';;\n  'session terminate') exit 0;;"),
            "",
        );
        let (output, listed) = run(&project.config, &["sessions", "list"]);
        assert!(output.status.success(), "{listed}");
        let sessions = listed["data"]["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 5);
        assert_eq!(sessions[0]["id"], CLIENT);
        assert_eq!(sessions[0]["number"], 8);
        assert_eq!(sessions[2]["excluded"], true);
        let (output, terminated) = run(&project.config, &["sessions", "terminate", "--all"]);
        assert!(output.status.success(), "{terminated}");
        assert_eq!(
            terminated["data"]["terminated_session_ids"],
            serde_json::json!([CLIENT, JOB])
        );
        let calls = std::fs::read_to_string(&project.calls).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|call| call.starts_with("session terminate "))
                .count(),
            2
        );
    }

    #[test]
    fn a_malformed_session_listing_refuses_before_any_termination() {
        let project = project("'session list') printf 'session : not-a-uuid\\n';;", "");
        let (output, error) = run(&project.config, &["sessions", "terminate", "--all"]);
        assert!(!output.status.success(), "{error}");
        assert!(
            error.to_string().contains("incomplete session list"),
            "{error}"
        );
        let calls = std::fs::read_to_string(&project.calls).unwrap();
        assert!(!calls.contains("session terminate"), "{calls}");
    }

    #[test]
    fn terminate_preserves_successful_ids_when_another_session_is_refused() {
        let listing = [
            session(CLIENT, 8, "1CV8C"),
            session(JOB, 9, "BackgroundJob"),
        ]
        .concat();
        let project = project(
            &format!(
                "'session list') printf '%s' '{listing}';;\n  'session terminate')\n    case \"$*\" in *{CLIENT}*) exit 0;; esac\n    printf 'permission denied\\n' >&2; exit 1;;"
            ),
            "",
        );
        let (output, result) = run(&project.config, &["sessions", "terminate", "--all"]);
        assert!(!output.status.success(), "{result}");
        assert_eq!(
            result["data"]["terminated_session_ids"],
            serde_json::json!([CLIENT])
        );
        assert_eq!(result["data"]["errors"][0]["session_id"], JOB);
        assert!(result.to_string().contains("permission denied"), "{result}");
    }

    #[test]
    fn lock_commands_name_the_missing_credential_level() {
        let project = project("*) exit 88;;", "    user: ib-admin\n");
        let (output, error) = run(&project.config, &["sessions", "deny"]);
        assert!(!output.status.success(), "{error}");
        assert!(
            error.to_string().contains("infobase.cluster.user"),
            "{error}"
        );
        let project = self::project("*) exit 88;;", "");
        let (output, error) = run(&project.config, &["sessions", "allow"]);
        assert!(!output.status.success(), "{error}");
        assert!(error.to_string().contains("infobase.user"), "{error}");
    }

    #[test]
    fn a_maintenance_window_restores_only_its_own_jobs_change() {
        let script = r#"'help infobase') printf '%s\n' '--infobase-user= --infobase-pwd= --denied-parameter=';;
  'infobase info')
    read denied < "$0.denied"
    read jobs < "$0.jobs"
    read marker < "$0.marker"
    read code < "$0.code"
    printf 'infobase : bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\nsessions-deny : %s\nscheduled-jobs-deny : %s\ndenied-parameter : %s\ndenied-message : \npermission-code : %s\n' "$denied" "$jobs" "$marker" "$code";;
  'infobase update')
    [ ! -f "$0.fail-update" ] || exit 77
    for arg; do
      case "$arg" in
        --sessions-deny=*) printf '%s\n' "${arg#*=}" > "$0.denied";;
        --scheduled-jobs-deny=*) printf '%s\n' "${arg#*=}" > "$0.jobs";;
        --denied-parameter=*) printf '%s\n' "${arg#*=}" > "$0.marker";;
        --permission-code=*) printf '%s\n' "${arg#*=}" > "$0.code";;
      esac
    done;;"#;
        let project = project(script, "    user: ib-admin\n    password: ib-secret\n");
        let overlay = project.config.with_file_name("v8project.local.yaml");
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(overlay)
            .unwrap()
            .write_all(b"      user: cluster-admin\n      password: cluster-secret\n")
            .unwrap();
        let bin = project.config.parent().unwrap().join("bin");
        std::fs::write(bin.join("rac.fail-update"), "").unwrap();
        for original_jobs in ["off", "on"] {
            std::fs::write(bin.join("rac.denied"), "off\n").unwrap();
            std::fs::write(bin.join("rac.marker"), "\n").unwrap();
            std::fs::write(bin.join("rac.code"), "\n").unwrap();
            std::fs::write(bin.join("rac.jobs"), format!("{original_jobs}\n")).unwrap();
            if original_jobs == "off" {
                let (output, failed) = run(
                    &project.config,
                    &["sessions", "deny", "--jobs", "--code", "private-code"],
                );
                assert!(!output.status.success(), "{failed}");
                let (output, failed) = run(&project.config, &["sessions", "allow"]);
                assert!(!output.status.success(), "{failed}");
                assert!(
                    failed.to_string().contains("unconfirmed outcome"),
                    "{failed}"
                );
                std::fs::remove_file(bin.join("rac.fail-update")).unwrap();
            }
            let (output, denied) = run(
                &project.config,
                &["sessions", "deny", "--jobs", "--code", "private-code"],
            );
            assert!(output.status.success(), "{denied}");
            assert_eq!(denied["data"]["sessions_denied"], true);
            assert_eq!(denied["data"]["jobs_denied"], true);
            for secret in ["private-code", "cluster-secret", "ib-secret"] {
                assert!(!denied.to_string().contains(secret));
                assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
            }
            let (output, allowed) = run(&project.config, &["sessions", "allow"]);
            assert!(output.status.success(), "{allowed}");
            assert_eq!(allowed["data"]["sessions_denied"], false);
            assert_eq!(allowed["data"]["jobs_denied"], original_jobs == "on");
            let (output, repeated) = run(&project.config, &["sessions", "allow"]);
            assert!(output.status.success(), "{repeated}");
            assert_eq!(repeated["data"]["changed"], false);
        }
        std::fs::write(bin.join("rac.denied"), "on\n").unwrap();
        std::fs::write(bin.join("rac.marker"), "foreign\n").unwrap();
        let before = std::fs::read(&project.calls).unwrap();
        let (output, error) = run(&project.config, &["sessions", "allow"]);
        assert!(!output.status.success(), "{error}");
        assert!(
            error.to_string().contains("foreign maintenance lock"),
            "{error}"
        );
        let after = std::fs::read(&project.calls).unwrap();
        assert!(!String::from_utf8_lossy(&after[before.len()..]).contains("infobase update"));
    }
}
