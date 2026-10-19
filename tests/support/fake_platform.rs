use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut output = None;
    let mut directory_output = None;
    let mut platform_log = None;

    if let Some(calls) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_CALLS") {
        let mut calls = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(calls)
            .expect("open fake platform call log");
        writeln!(calls, "{arguments:?}").expect("record fake platform call");
    }

    for pair in arguments.windows(2) {
        let option = pair[0].to_string_lossy();
        if matches!(option.as_ref(), "/DumpCfg" | "/DumpDBCfg" | "/DumpIB") {
            output = Some(pair[1].as_os_str());
        } else if option == "/DumpConfigToFiles" {
            directory_output = Some(pair[1].as_os_str());
        } else if option == "/Out" {
            platform_log = Some(pair[1].as_os_str());
        }
    }

    let ibcmd_generation = arguments
        .windows(2)
        .any(|pair| pair[0] == "config" && pair[1] == "generation-id");
    let designer_generation = arguments
        .iter()
        .any(|argument| argument == "/GetConfigGenerationID");
    if ibcmd_generation || designer_generation {
        let token = b"1111111111111111111111111111111111111111\n";
        if ibcmd_generation {
            std::io::stdout()
                .write_all(token)
                .expect("generation response");
        } else if let Some(platform_log) = platform_log {
            write_artifact(Path::new(platform_log), token);
        }
        return;
    }
    if arguments
        .windows(2)
        .any(|pair| pair[0] == "config" && pair[1] == "export")
    {
        directory_output = arguments.last().map(|argument| argument.as_os_str());
    }

    if let Some(directory) = directory_output {
        let directory = Path::new(directory);
        write_artifact(
            directory.join("Configuration.xml").as_path(),
            b"<Configuration/>\n",
        );
        write_artifact(
            directory.join("Module.bsl").as_path(),
            b"Procedure Published()\nEndProcedure\n",
        );
        write_artifact(
            directory.join("ConfigDumpInfo.xml").as_path(),
            b"<ConfigDumpInfo version=\"2.20\"/>\n",
        );
        if let Some(platform_log) = platform_log {
            write_artifact(Path::new(platform_log), b"platform log");
        }
        return;
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
