#![cfg(any(windows, target_os = "linux"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;
use tempfile::{tempdir, TempDir};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    root: TempDir,
    config: PathBuf,
    base: PathBuf,
    ibsrv: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().expect("temporary project");
        let platform = root.path().join("platform");
        fs::create_dir(&platform).expect("platform dir");
        let base = root.path().join("file base");
        fs::create_dir(&base).expect("file base");
        fs::write(base.join("1Cv8.1CD"), "database marker").expect("marker");
        let ibcmd = compile_fixture("fake_extension_ibcmd.rs", "ibcmd", &platform);
        compile_fixture("fake_platform.rs", "1cv8", &platform);
        let ibsrv = compile_fixture("fake_ibsrv.rs", "ibsrv", root.path());
        let source = root.path().join("src");
        fs::create_dir(&source).expect("source dir");
        fs::write(source.join("Configuration.xml"), "<Configuration/>").expect("configuration");
        let config = root.path().join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "format: DESIGNER\nproviders:\n  download: ibcmd\n  infobase.create: ibcmd\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: '{}'\n",
                base.display(),
                ibcmd.display(),
            ),
        )
        .expect("config");
        Self {
            root,
            config,
            base,
            ibsrv,
        }
    }

    fn serve(&self, base: &Path) -> Server {
        Server(
            Command::new(&self.ibsrv)
                .arg("--database-path")
                .arg(base)
                .stdin(Stdio::piped())
                .spawn()
                .expect("start fake ibsrv"),
        )
    }

    fn run(&self, args: &[&str]) -> (std::process::ExitStatus, Value) {
        let output = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
            .current_dir(self.root.path())
            .env("V8R_FAKE_DESIGNER_DIR", self.root.path())
            .env(
                "V8_RUNNER_FAKE_PLATFORM_CALLS",
                self.root.path().join("designer.log"),
            )
            .arg("--config")
            .arg(&self.config)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run v8-runner");
        let envelope = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{error}: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status, envelope)
    }

    fn calls(&self) -> PathBuf {
        self.root.path().join("ibcmd-args.log")
    }
}

fn compile_fixture(source: &str, stem: &str, directory: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support")
        .join(source);
    let binary = directory.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new("rustc")
        .arg("--edition=2021")
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile fake tool");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    binary
}

#[test]
fn a_running_ibsrv_blocks_direct_download_and_extension_inventory() {
    let fixture = Fixture::new();
    let alias = fixture.root.path().join("alias with spaces");
    #[cfg(windows)]
    junction::create(&fixture.base, &alias).expect("base junction");
    #[cfg(target_os = "linux")]
    std::os::unix::fs::symlink(&fixture.base, &alias).expect("base symlink");
    let mut server = fixture.serve(&alias);

    let (status, preview) = fixture.run(&["download", "main", "--output", "probe.cf", "--dry-run"]);
    assert!(status.success(), "{preview}");
    assert_eq!(preview["data"]["provider_dispatched"], false, "{preview}");

    fs::write(fixture.root.path().join("probe.cfe"), b"package").expect("extension package");
    for args in [
        &["download", "main", "--output", "probe.cf"][..],
        &["extensions", "list"][..],
        &["upload", "probe.cfe", "--ref", "Probe", "--mode", "load"][..],
        &["infobase", "create", "--recreate"][..],
    ] {
        let (status, envelope) = fixture.run(args);
        assert!(
            !fixture.calls().exists(),
            "{args:?} dispatched ibcmd: {envelope}"
        );
        assert_eq!(status.code(), Some(3), "{envelope}");
        assert_eq!(envelope["error"]["code"], "infobase_busy", "{envelope}");
        match args[0] {
            "extensions" => {
                // Inventory uses the common error form before any platform work.
                assert!(envelope["data"]["message"].is_string(), "{envelope}");
            }
            "upload" => {
                assert_eq!(envelope["data"]["provider_dispatched"], false, "{envelope}");
            }
            "download" => {
                assert_eq!(envelope["data"]["target_state"], "unchanged", "{envelope}");
            }
            "infobase" => {
                assert_eq!(
                    envelope["data"]["removed"],
                    serde_json::json!([]),
                    "{envelope}"
                );
                assert_eq!(
                    fs::read(fixture.base.join("1Cv8.1CD")).unwrap(),
                    b"database marker"
                );
            }
            _ => unreachable!(),
        }
        let message = envelope["error"]["message"].as_str().expect("message");
        assert!(
            message.contains(&format!("ibsrv pid {}", server.0.id())),
            "{envelope}"
        );
        assert!(!fixture.root.path().join("probe.cf").exists());
        assert!(!fixture.root.path().join("designer.log").exists());
        assert!(server.0.try_wait().expect("server status").is_none());
    }
}

#[test]
fn a_running_ibsrv_for_another_base_leaves_direct_download_available() {
    let fixture = Fixture::new();
    let other = fixture.root.path().join("other base");
    fs::create_dir(&other).expect("other file base");
    fs::write(other.join("1Cv8.1CD"), "other marker").expect("other base marker");
    let mut server = fixture.serve(&other);

    let (status, envelope) = fixture.run(&["download", "main", "--output", "probe.cf"]);
    assert!(status.success(), "{envelope}");
    assert_eq!(envelope["data"]["published"], true, "{envelope}");
    assert!(fixture.calls().exists());
    assert!(fixture.root.path().join("probe.cf").is_file());
    assert!(server.0.try_wait().expect("server status").is_none());
}

#[test]
fn an_ibsrv_over_the_same_data_file_under_another_directory_blocks_download() {
    let fixture = Fixture::new();
    let alias = fixture.root.path().join("other directory");
    fs::create_dir(&alias).expect("alias directory");
    fs::hard_link(fixture.base.join("1Cv8.1CD"), alias.join("1Cv8.1CD")).expect("data file alias");
    let server = fixture.serve(&alias);

    let (status, envelope) = fixture.run(&["download", "main", "--output", "probe.cf"]);
    assert_eq!(status.code(), Some(3), "{envelope}");
    assert_eq!(envelope["error"]["code"], "infobase_busy", "{envelope}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(
        message.contains(&format!("ibsrv pid {}", server.0.id())),
        "{envelope}"
    );
    assert!(!fixture.calls().exists());
    assert!(!fixture.root.path().join("probe.cf").exists());
}
