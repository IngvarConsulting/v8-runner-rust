use std::path::Path;
use std::time::{Duration, Instant};

fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut output = None;
    let mut platform_log = None;

    for pair in arguments.windows(2) {
        let option = pair[0].to_string_lossy();
        if matches!(option.as_ref(), "/DumpCfg" | "/DumpDBCfg" | "/DumpIB") {
            output = Some(pair[1].as_os_str());
        } else if option == "/Out" {
            platform_log = Some(pair[1].as_os_str());
        }
    }

    let Some(output) = output else {
        eprintln!("fake platform did not receive a supported export target");
        std::process::exit(2);
    };

    if let Some(ready_path) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_READY") {
        write_artifact(Path::new(&ready_path), b"ready");
    }
    if let Some(release_path) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_RELEASE") {
        let release_path = Path::new(&release_path);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !release_path.is_file() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for fake platform release at {}",
                release_path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    write_artifact(Path::new(output), b"payload");
    if let Some(platform_log) = platform_log {
        write_artifact(Path::new(platform_log), b"platform log");
    }
}

fn write_artifact(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create artifact parent");
    }
    std::fs::write(path, contents).expect("write fake platform artifact");
}
