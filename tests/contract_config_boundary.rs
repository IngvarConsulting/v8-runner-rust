//! Неподдержанное сочетание в конфиге отклоняется до того, как запускается любая
//! утилита: цена ошибки не должна включать частично сделанную работу.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

struct Project {
    config: PathBuf,
    calls: PathBuf,
}

fn write_project(dir: &Path, yaml_tail: &str) -> Project {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    let calls = dir.join("calls.log");
    for utility in ["1cv8", "ibcmd", "1cedtcli"] {
        write_shell_script(
            &install_dir.join("bin").join(utility),
            &format!(
                "printf '{utility} %s\\n' \"$*\" >> '{}'\nexit 0",
                calls.display()
            ),
        );
    }
    let config = dir.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: {}\ntools:\n  platform:\n    path: {}\n  edt_cli:\n    path: {}\n{yaml_tail}",
            work_path.display(),
            install_dir.display(),
            install_dir.join("bin").join("1cedtcli").display()
        ),
    )
    .expect("write config");
    Project { config, calls }
}

/// Каждое сочетание ниже — ошибка конфига, и ни одно из них не доходит до платформы.
#[test]
fn an_unsupported_combination_is_refused_before_any_utility_runs() {
    let ib = "File=/tmp/ib";
    let cases: [(&str, String); 4] = [
        (
            "dbms on a file infobase",
            format!("format: DESIGNER\ninfobase:\n  connection: '{ib}'\n  dbms:\n    kind: PostgreSQL\n    server: db\n    name: demo\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n"),
        ),
        (
            "a cluster section on a file infobase",
            format!("format: DESIGNER\ninfobase:\n  connection: '{ib}'\n  cluster:\n    ras: srv:1545\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n"),
        ),
        (
            "EDT format over a Designer-layout source set",
            format!("format: EDT\ninfobase:\n  connection: '{ib}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n"),
        ),
        (
            "an override for an operation without a choice",
            format!("format: DESIGNER\nproviders:\n  load: designer\ninfobase:\n  connection: '{ib}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n"),
        ),
    ];

    for (why, yaml) in cases {
        let dir = temp_workspace();
        let project = write_project(dir.path(), &yaml);
        for command in [
            vec!["push"],
            vec!["pull", "--mode", "full"],
            vec!["infobase", "create"],
        ] {
            let output = v8_runner_command()
                .args([
                    "--config",
                    &project.config.display().to_string(),
                    "--json-message",
                ])
                .args(&command)
                .output()
                .expect("run command");
            let payload: Value = serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|_| panic!("{why}: no envelope for {command:?}"));
            assert!(
                !output.status.success(),
                "{why}: `{command:?}` succeeded: {payload}"
            );
            assert_eq!(payload["error"]["kind"], "validation", "{why}: {payload}");
            assert!(
                !project.calls.exists(),
                "{why}: `{command:?}` reached the platform before validation refused"
            );
        }
    }
}
