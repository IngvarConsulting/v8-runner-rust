use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::tempdir;

fn compile_fake_designer(target_dir: &Path) -> PathBuf {
    fs::create_dir_all(target_dir).expect("platform dir");
    let target = target_dir.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_platform.rs");
    let compile = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&target)
        .output()
        .expect("run rustc for fake platform");
    assert!(
        compile.status.success(),
        "failed to compile {}: {}",
        source.display(),
        String::from_utf8_lossy(&compile.stderr)
    );

    target
}

fn write_config(root: &Path, platform: &Path) -> PathBuf {
    let project = root.join("project");
    let work = root.join("work");
    let infobase = root.join("ib");
    fs::create_dir_all(&project).expect("project");
    fs::create_dir_all(&work).expect("work");
    fs::create_dir_all(&infobase).expect("infobase");
    fs::write(infobase.join("1Cv8.1CD"), "database").expect("infobase file");
    let config = root.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set: []\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            infobase.display(),
            platform.display(),
        ),
    )
    .expect("config");
    config
}

fn run_json(config: &Path, arguments: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(config)
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run v8-runner");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON envelope")
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

struct BlockedProcess {
    child: Option<std::process::Child>,
    release: PathBuf,
}

impl BlockedProcess {
    fn new(child: std::process::Child, release: PathBuf) -> Self {
        Self {
            child: Some(child),
            release,
        }
    }

    fn release_and_wait(mut self) -> std::process::Output {
        fs::write(&self.release, b"release").expect("release fake platform");
        self.child
            .take()
            .expect("blocked child")
            .wait_with_output()
            .expect("wait for blocked v8-runner")
    }
}

impl Drop for BlockedProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = fs::write(&self.release, b"release");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn designer_configuration_and_dt_exports_publish_through_native_cli() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    let cfe = root.path().join("project/dist/sales.cfe");
    let dt = root.path().join("project/dist/base.dt");

    let cfe_json = run_json(
        &config,
        &[
            "infobase",
            "configuration",
            "export",
            "--state",
            "database",
            "--extension",
            "Sales",
            "--output",
            cfe.to_str().expect("cfe path"),
        ],
    );
    assert_eq!(cfe_json["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(cfe_json["data"]["artifact_kind"], "cfe");
    assert_eq!(cfe_json["data"]["published"], true);
    assert_eq!(fs::read(&cfe).expect("published CFE"), b"payload");

    let dt_json = run_json(
        &config,
        &[
            "infobase",
            "dump",
            "--output",
            dt.to_str().expect("dt path"),
        ],
    );
    assert_eq!(dt_json["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(dt_json["data"]["artifact_kind"], "dt");
    assert_eq!(dt_json["data"]["published"], true);
    assert_eq!(fs::read(&dt).expect("published DT"), b"payload");
}

#[test]
fn concurrent_native_cli_processes_observe_the_workspace_lock() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    let ready = root.path().join("first-platform.ready");
    let release = root.path().join("first-platform.release");
    let first_dt = root.path().join("project/dist/first.dt");
    let second_dt = root.path().join("project/dist/second.dt");

    let first = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .arg("--json-message")
        .args(["infobase", "dump", "--output"])
        .arg(&first_dt)
        .env("V8_RUNNER_FAKE_PLATFORM_READY", &ready)
        .env("V8_RUNNER_FAKE_PLATFORM_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first v8-runner");
    let first = BlockedProcess::new(first, release.clone());
    wait_for_file(&ready);

    let second = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .arg("--json-message")
        .args(["infobase", "dump", "--output"])
        .arg(&second_dt)
        .output()
        .expect("run contending v8-runner");
    assert!(!second.status.success(), "contender unexpectedly succeeded");
    let contender: Value = serde_json::from_slice(&second.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid contender JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        )
    });
    assert_eq!(contender["error"]["code"], "workspace_busy");
    assert_eq!(contender["steps"][0]["name"], "workspace lock");
    assert_eq!(contender["steps"][0]["kind"], "prepare_workspace");
    assert!(!second_dt.exists());

    let first_output = first.release_and_wait();
    assert!(
        first_output.status.success(),
        "first v8-runner failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );
    assert_eq!(fs::read(first_dt).expect("first DT"), b"payload");
}
