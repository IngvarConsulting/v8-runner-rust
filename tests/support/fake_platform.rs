use std::io::{Read, Write};
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
        if let Some(gate) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_DUMP_GATE") {
            let address = gate.to_str().expect("own writer gate address");
            let mut gate = std::net::TcpStream::connect(address).expect("connect own writer gate");
            gate.set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            gate.set_write_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            // EOF is deliberately earlier than process exit: an EOF-only wait
            // must not make this live finite writer eligible for publication.
            close_output_pipes().expect("close finite writer stdout/stderr");
            gate.write_all(&std::process::id().to_le_bytes())
                .expect("held writer PID");
            gate.flush().expect("flush writer identity");
            let mut release = [0];
            // Disconnect on test failure also ends this bounded fixture, never a detached writer.
            gate.read_exact(&mut release)
                .expect("finite writer release");
            assert_eq!(release, [1]);
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

#[cfg(unix)]
fn close_output_pipes() -> std::io::Result<()> {
    unsafe extern "C" {
        fn close(fd: i32) -> i32;
    }
    for fd in [1, 2] {
        // SAFETY: this single-threaded fixture owns its standard descriptors;
        // all intended writes are complete. The TCP gate was opened separately.
        if unsafe { close(fd) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn close_output_pipes() -> std::io::Result<()> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn SetStdHandle(kind: u32, handle: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let kinds = [(-11_i32) as u32, (-12_i32) as u32];
    let mut handles = [std::ptr::null_mut(); 2];
    for (kind, slot) in kinds.into_iter().zip(&mut handles) {
        // SAFETY: kind is Win32 STD_OUTPUT_HANDLE or STD_ERROR_HANDLE.
        let handle = unsafe { GetStdHandle(kind) };
        if handle.is_null() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fixture standard pipe missing",
            ));
        }
        if handle == (-1_isize) as *mut c_void {
            return Err(std::io::Error::last_os_error());
        }
        *slot = handle;
    }
    for kind in kinds {
        // SAFETY: detach only this fixture's standard-handle table entries;
        // keep their owned values above until CloseHandle below.
        if unsafe { SetStdHandle(kind, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    // A launcher may alias the two standard entries; each owned handle closes once.
    let unique = if handles[0] == handles[1] {
        &handles[..1]
    } else {
        &handles[..]
    };
    for &handle in unique {
        // SAFETY: these are valid process-owned handles, detached above, and
        // all intended writes are complete. The TCP gate is a different handle.
        if unsafe { CloseHandle(handle) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn close_output_pipes() -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "fixture standard pipe closure",
    ))
}
