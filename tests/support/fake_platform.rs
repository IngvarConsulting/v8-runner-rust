use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut output = None;
    let mut directory_output = None;
    let mut platform_log = None;

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

    // Unit tests run the use case in-process. Each installed executable carries
    // its own scenario, so parallel tests need no process-wide environment changes.
    let scenario = std::env::current_exe()
        .expect("fake platform executable path")
        .with_extension("dump-fixture");
    if scenario.is_dir() {
        run_dump_fixture(
            &scenario,
            &arguments,
            directory_output.map(Path::new),
            platform_log.map(Path::new),
        );
        return;
    }

    if let Some(calls) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_CALLS") {
        let mut calls = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(calls)
            .expect("open fake platform call log");
        writeln!(calls, "{arguments:?}").expect("record fake platform call");
    }

    let checking = arguments.iter().any(|arg| arg == "/CheckConfig");
    let restoring = arguments.iter().any(|arg| arg == "/RestoreIB");
    if std::env::var_os("V8_RUNNER_FAKE_PLATFORM_FAIL_WITH_PASSWORD").is_some()
        && (directory_output.is_some() || checking || restoring)
    {
        let password = arguments
            .windows(2)
            .find(|pair| pair[0] == "/P")
            .expect("password from Designer argv")[1]
            .to_str()
            .expect("UTF-8 test password");
        let log = platform_log.expect("Designer /Out");
        if checking {
            let first_line = password.lines().next().expect("first password line").trim();
            let suffix = if first_line.contains(char::is_whitespace) {
                ""
            } else {
                first_line
            };
            write_artifact(
                Path::new(log),
                format!(
                    "{{CommonModules.TestModule{suffix}(4,2)}}: compile error: {password}\n\
                     Catalogs.Demo{suffix} object error: {password}\n"
                )
                .as_bytes(),
            );
            eprintln!("Designer stderr rejected [{password}]");
            std::process::exit(101);
        }
        if restoring {
            let user = &arguments
                .windows(2)
                .find(|pair| pair[0] == "/N")
                .expect("user from Designer argv")[1];
            eprintln!("Designer rejected user {}", user.to_string_lossy());
        }
        write_artifact(
            Path::new(log),
            format!(
                "Designer /Out authentication rejected [{password}] DBPwd=fake-database-secret;SPwd=fake-cluster-secret;\n"
            )
            .as_bytes(),
        );
        println!("Designer stdout rejected [{password}]");
        eprintln!("Designer stderr rejected [{password}]");
        std::process::exit(1);
    }
    if restoring {
        if let Some(platform_log) = platform_log {
            write_artifact(Path::new(platform_log), b"platform log");
        }
        return;
    }

    let ibcmd_generation = ibcmd_config_args(&arguments, "generation-id").is_some();
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
    if ibcmd_config_args(&arguments, "export").is_some() {
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

    if let Some(marker) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_CONSOLE_EVENT") {
        record_console_events(marker.into());
    }
    if let Some(pid_path) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_PID") {
        write_artifact(
            Path::new(&pid_path),
            std::process::id().to_string().as_bytes(),
        );
    }
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

fn ibcmd_config_args<'a>(arguments: &'a [OsString], command: &str) -> Option<&'a [OsString]> {
    let start = arguments
        .windows(2)
        .position(|pair| pair[0] == "config" && pair[1] == command)?;
    Some(&arguments[start + 2..])
}

fn run_dump_fixture(
    scenario: &Path,
    arguments: &[OsString],
    designer_target: Option<&Path>,
    platform_log: Option<&Path>,
) {
    let joined = arguments
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let calls = fs::read_to_string(scenario.join("calls-path")).expect("call log path");
    let mut calls = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(calls)
        .expect("open dump call log");
    writeln!(calls, "{joined}").expect("record dump call");
    if let Some(log) = platform_log {
        write_artifact(log, format!("designer log for {joined}\n").as_bytes());
    }

    // These scenarios exercise dumps without a readable generation.
    if arguments.iter().any(|arg| arg == "/GetConfigGenerationID")
        || ibcmd_config_args(arguments, "generation-id").is_some()
    {
        return;
    }
    let ibcmd_export = ibcmd_config_args(arguments, "export");
    if ibcmd_export
        .and_then(|args| args.first())
        .is_some_and(|arg| arg == "status")
    {
        if let Some(forecast) = fixture_option(scenario, "forecast") {
            print!("{forecast}");
        }
        return;
    }
    if arguments
        .iter()
        .any(|arg| arg == "/DumpExternalDataProcessorOrReportToFiles")
    {
        // This scenario exercises a successful process that left no descriptor.
        return;
    }
    let target = designer_target.or_else(|| ibcmd_export?.last().map(Path::new));
    let Some(target) = target else {
        panic!("unsupported fake dump command: {joined}");
    };
    if let Some(refusal) = fixture_option(scenario, "refusal") {
        let (code, flag) = refusal
            .split_once('\n')
            .expect("exit code and refusal flag");
        if arguments.iter().any(|arg| {
            arg == flag || flag.ends_with('=') && arg.to_string_lossy().starts_with(flag)
        }) {
            std::process::exit(code.parse().expect("fake platform exit code"));
        }
    }
    fs::create_dir_all(target).expect("create fake dump target");
    if scenario.join("staged").is_file() {
        if fs::read_dir(target)
            .expect("read fake dump target")
            .next()
            .is_some()
        {
            eprintln!("fake platform requires an empty export directory");
            std::process::exit(255);
        }
        write_artifact(&target.join("Configuration.xml"), b"new\n");
        write_artifact(&target.join("Catalogs/Items.xml"), b"item\n");
        write_artifact(
            &target.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"2.20\"/>\n",
        );
    }
}

fn fixture_option(scenario: &Path, name: &str) -> Option<String> {
    match fs::read_to_string(scenario.join(name)) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => panic!("read dump fixture option {name}: {error}"),
    }
}

/// Событие консоли (`CTRL_BREAK`, `CTRL_C`) этот процесс переживает и отмечает файлом
/// `marker`: так тест видит, что событие до потомка раннера дошло. Обработчик пишет отметку
/// первым делом, а снимает процесс раннер.
#[cfg(windows)]
fn record_console_events(marker: std::path::PathBuf) {
    static MARKER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn record(kind: u32) -> i32 {
        if let Some(marker) = MARKER.get() {
            let _ = std::fs::write(marker, format!("console event {kind}"));
        }
        1
    }

    MARKER.set(marker).expect("one console event marker");
    // SAFETY: the handler is a plain function that lives for the whole process.
    assert_ne!(
        unsafe { SetConsoleCtrlHandler(Some(record), 1) },
        0,
        "install the console event handler"
    );
}

#[cfg(not(windows))]
fn record_console_events(_marker: std::path::PathBuf) {}

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
