//! `ibcmd` у базы с пользователями без объявленного пользователя (#742).
//!
//! Поддельный `ibcmd` повторяет замер: пояснение, затем «Имя пользователя: » без конца на
//! закрытом вводе. Команда отказывает за секунды отказом окружения со слоем `infobase_user`,
//! а процесс двойника снят
//! (`INV.PLATFORM.A-REPEATED-IBCMD-QUESTION-IS-AN-INFOBASE-USER-REFUSAL`).
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Project {
    /// Файловая база `ib` без пользователя в местном слое; `pull`, `push` и `apply` идут
    /// через `ibcmd`, который спрашивает имя пользователя без конца.
    fn new() -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::create_dir_all(root.join("ib")).expect("infobase");
        fs::write(root.join("ib").join("1Cv8.1CD"), "database").expect("base file");
        write_shell_script(
            &root.join("ibcmd"),
            &format!(
                r#"printf '%s\n' "$$" >> '{pids}'
printf 'Для выполнения операции требуется аутентификация в информационной базе\n'
while :; do printf 'Имя пользователя: '; done"#,
                pids = root.join("pids").display(),
            ),
        );
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\nproviders:\n  push: ibcmd\n  pull: ibcmd\n  apply: ibcmd\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                root.join("ibcmd").display()
            ),
        )
        .expect("config");
        fs::write(
            root.join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n",
        )
        .expect("local layer");
        Self { dir, config }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn run(&self, args: &[&str]) -> (Output, Duration) {
        let started = Instant::now();
        let output = v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run CLI");
        (output, started.elapsed())
    }

    /// Номера процессов, которые запускал двойник.
    fn pids(&self) -> Vec<String> {
        fs::read_to_string(self.root().join("pids"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .expect("kill -0")
        .success()
}

/// Отказ `infobase_user` за секунды, ни одного живого двойника после команды.
fn assert_refused_quickly(project: &Project, args: &[&str]) {
    let (output, elapsed) = project.run(args);
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });

    assert!(elapsed < Duration::from_secs(30), "{args:?}: {elapsed:?}");
    assert_eq!(output.status.code(), Some(2), "{args:?}: {payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{args:?}: {payload}"
    );
    assert_eq!(payload["error"]["layer"], "infobase_user", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("Имя пользователя:"), "{message}");
    assert!(message.contains("infobases.<name>.user"), "{message}");
    assert!(message.contains("v8project.local.yaml"), "{message}");
    let pids = project.pids();
    assert!(!pids.is_empty(), "{args:?}: ibcmd was asked");
    let deadline = Instant::now() + Duration::from_secs(5);
    while pids.iter().any(|pid| alive(pid)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        pids.iter().all(|pid| !alive(pid)),
        "{args:?}: the asking ibcmd is gone: {pids:?}"
    );
}

#[test]
fn pull_force_refuses_an_infobase_that_asks_for_its_user() {
    assert_refused_quickly(&Project::new(), &["pull", "--force"]);
}

#[test]
fn push_force_refuses_an_infobase_that_asks_for_its_user() {
    assert_refused_quickly(&Project::new(), &["push", "--force"]);
}
