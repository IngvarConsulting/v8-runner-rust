use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::launch::LaunchVia;
use crate::domain::runner::LaunchClientModeRequest;
use crate::domain::runner::{launch_key_alias_matches, LaunchOptions};
use crate::platform::connection::V8Connection;
use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessRunner,
};
use crate::platform::result::PlatformCommandResult;

#[derive(Debug, Error)]
pub enum EnterpriseError {
    #[error("failed to execute enterprise process: {0}")]
    Spawn(ProcessError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchClientMode {
    Designer,
    Thin,
    Thick,
    Ordinary,
}

pub struct EnterpriseDsl<'a> {
    binary: PathBuf,
    connection: V8Connection,
    /// Каким адресом клиент открывает базу.
    address: ClientAddress,
    additional_launch_keys: Vec<String>,
    client_mode: LaunchClientMode,
    runner: &'a dyn ProcessRunner,
    log_file: PathBuf,
    execution_policy: ProcessExecutionPolicy,
}

impl<'a> EnterpriseDsl<'a> {
    pub fn new(
        binary: PathBuf,
        connection: V8Connection,
        address: ClientAddress,
        additional_launch_keys: Vec<String>,
        client_mode: LaunchClientMode,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
        execution_policy: ProcessExecutionPolicy,
    ) -> Self {
        Self {
            binary,
            connection,
            address,
            additional_launch_keys,
            client_mode,
            runner,
            log_file,
            execution_policy,
        }
    }

    pub fn run_launch(
        &self,
        launch: &LaunchOptions,
    ) -> Result<PlatformCommandResult, EnterpriseError> {
        let process = self
            .runner
            .run_with_policy(
                &self.build_command(launch).into_request(self.binary.clone()),
                &self.execution_policy,
            )
            .map_err(EnterpriseError::Spawn)?;

        let (platform_log_path, platform_log, platform_log_read_error) =
            match std::fs::read_to_string(&self.log_file) {
                Ok(contents) => (Some(self.log_file.clone()), Some(contents), None),
                Err(error) => (
                    Some(self.log_file.clone()),
                    None,
                    Some(format!(
                        "failed to read enterprise /Out log '{}': {error}",
                        self.log_file.display()
                    )),
                ),
            };

        Ok(PlatformCommandResult {
            process,
            platform_log_path,
            platform_log,
            platform_log_read_error,
        })
    }

    fn build_command(&self, launch: &LaunchOptions) -> LaunchCommand {
        let mut launch = launch.clone();
        launch.internal_out = Some(self.log_file.display().to_string());
        build_launch_command(
            self.client_mode,
            &self.address,
            &self.connection,
            &self.additional_launch_keys,
            &launch,
        )
    }
}

impl From<LaunchClientModeRequest> for LaunchClientMode {
    fn from(value: LaunchClientModeRequest) -> Self {
        match value {
            LaunchClientModeRequest::Designer => LaunchClientMode::Designer,
            LaunchClientModeRequest::Thin => LaunchClientMode::Thin,
            LaunchClientModeRequest::Thick => LaunchClientMode::Thick,
            LaunchClientModeRequest::Ordinary => LaunchClientMode::Ordinary,
        }
    }
}

/// Каким адресом клиент открывает базу. Реквизиты базы берутся из строки подключения, а
/// идут ли они, говорит сам вариант адреса: решение принимает выбор адреса клиента в
/// `use_cases::client_address`, в командную строку его переводит только [`Self::args`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAddress {
    /// `infobase.connection` вместе с `/N` и `/P`.
    Connection,
    /// `infobase.web.url` как ws-соединение с `/N` и `/P`.
    Web(String),
    /// `infobase.web.url` как ws-соединение без `/N` и `/P`: приём реквизитов по `/WS`
    /// автономного сервера не замерен (#184).
    WebWithoutCredentials(String),
}

impl ClientAddress {
    /// Каким адресом открыта база — так его называет ответ.
    pub const fn via(&self) -> LaunchVia {
        match self {
            Self::Connection => LaunchVia::Connection,
            Self::Web(_) | Self::WebWithoutCredentials(_) => LaunchVia::Web,
        }
    }

    /// Клиентский адрес, если выбран он.
    pub fn web_url(&self) -> Option<&str> {
        match self {
            Self::Connection => None,
            Self::Web(url) | Self::WebWithoutCredentials(url) => Some(url),
        }
    }

    /// Адрес в командной строке клиента; реквизиты — из `connection`.
    fn args(&self, connection: &V8Connection) -> Vec<String> {
        match self {
            Self::Connection => connection.args(),
            Self::Web(url) => {
                let mut args = vec!["/WS".to_owned(), url.clone()];
                args.extend(connection.credential_args());
                args
            }
            Self::WebWithoutCredentials(url) => vec!["/WS".to_owned(), url.clone()],
        }
    }
}

/// Client arguments together with the indexes of those already encoded for the
/// 1C parser, which must reach the client verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    pub args: Vec<String>,
    pub verbatim_args: Vec<usize>,
}

impl LaunchCommand {
    /// A request that carries the verbatim indexes along with the arguments.
    pub fn into_request(self, program: PathBuf) -> ProcessRequest {
        ProcessRequest {
            program,
            args: self.args,
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
            windows_verbatim_args: self.verbatim_args,
        }
    }
}

#[cfg(test)]
pub fn build_launch_args(
    mode: LaunchClientMode,
    address: &ClientAddress,
    connection: &V8Connection,
    additional_launch_keys: &[String],
    launch: &LaunchOptions,
) -> Vec<String> {
    build_launch_command(mode, address, connection, additional_launch_keys, launch).args
}

pub fn build_launch_command(
    mode: LaunchClientMode,
    address: &ClientAddress,
    connection: &V8Connection,
    additional_launch_keys: &[String],
    launch: &LaunchOptions,
) -> LaunchCommand {
    let mut args = vec![match mode {
        LaunchClientMode::Designer => "DESIGNER",
        LaunchClientMode::Thin | LaunchClientMode::Thick | LaunchClientMode::Ordinary => {
            "ENTERPRISE"
        }
    }
    .to_owned()];
    args.push("/DisableStartupDialogs".to_owned());
    args.extend(address.args(connection));
    if matches!(mode, LaunchClientMode::Ordinary) {
        args.push("/RunModeOrdinaryApplication".to_owned());
    }
    if launch.use_privileged_mode {
        args.push("/UsePrivilegedMode".to_owned());
    }
    if let Some(execute) = &launch.execute {
        args.push("/Execute".to_owned());
        args.push(execute.clone());
    }
    let mut verbatim_args = Vec::new();
    if let Some(c) = &launch.c {
        args.push("/C".to_owned());
        verbatim_args.push(args.len());
        args.push(encode_launch_parameter(c));
    }

    let mut extra_args = Vec::new();
    if !matches!(mode, LaunchClientMode::Designer) {
        extra_args.extend(filtered_raw_launch_args(additional_launch_keys));
    }
    extra_args.extend(filtered_raw_launch_args(&launch.raw_args));
    args.extend(extra_args);

    if let Some(out) = effective_out_path(launch) {
        args.push("/Out".to_owned());
        args.push(out.to_owned());
    }
    LaunchCommand {
        args,
        verbatim_args,
    }
}

/// Encodes a `/C` value for the 1C command-line parser, so that the client
/// receives it as exactly one launch parameter and no part of it can become
/// another launch key. 1C reads `""` inside a quoted value as one quote.
///
/// Windows: the client parses its raw command line, so the value is always
/// framed and must reach it verbatim (see [`LaunchCommand::verbatim_args`]).
/// macOS and Linux: the platform rebuilds a command line from argv and frames
/// only an argument that contains an ASCII space itself; any other value —
/// empty, with a tab or a line break — is framed here (measured on macOS
/// 8.3.27, `references/1c/confirmed-runtime-measurements.md`).
pub fn encode_launch_parameter(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    if cfg!(windows) || !value.contains(' ') {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

pub fn normalize_launch_payload_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn effective_out_path(launch: &LaunchOptions) -> Option<&str> {
    launch.internal_out.as_deref().or(launch.out.as_deref())
}

fn filtered_raw_launch_args(args: &[String]) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if let Some((reserved, consumes_value)) = reserved_launch_key(arg) {
            if reserved {
                skip_value = consumes_value;
                continue;
            }
        }
        filtered.push(arg.clone());
    }
    filtered
}

// Известный предел: ключей соединения здесь нет. Пользовательский `/WS` или
// `/IBConnectionString` из `additional-launch-keys` допишется после нашего, и какой из двух
// возьмёт платформа, решает она сама. Резервирование ключей соединения — отдельный предмет.
fn reserved_launch_key(arg: &str) -> Option<(bool, bool)> {
    let reserved_key = [
        "c",
        "execute",
        "useprivilegedmode",
        "out",
        "runmodeordinaryapplication",
        "disablestartupdialogs",
    ]
    .into_iter()
    .find(|key| launch_key_alias_matches(arg, key))?;
    let normalized = arg
        .trim_start()
        .trim_start_matches(['/', '-'])
        .trim()
        .to_ascii_lowercase();
    let consumes_value =
        normalized == reserved_key && matches!(reserved_key, "c" | "execute" | "out");
    Some((true, consumes_value))
}

#[cfg(test)]
mod tests {
    use super::{
        build_launch_args, build_launch_command, encode_launch_parameter,
        normalize_launch_payload_path, ClientAddress, EnterpriseDsl, LaunchClientMode,
    };
    use crate::domain::runner::LaunchOptions;
    use crate::platform::connection::V8Connection;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessExecutor, ProcessRequest, ProcessResult,
        ProcessRunner, SpawnResult, WorkGiven,
    };
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn c_payload_uses_forward_slashes() {
        let normalized =
            normalize_launch_payload_path(Path::new("C:\\tmp\\path with space\\cfg.json"));
        assert_eq!(normalized, "C:/tmp/path with space/cfg.json");
    }

    /// Форма argv веб-пути: `/WS` с голым адресом вместо строки подключения. Живой
    /// прогон на платформе — приёмка у владельца, но форма закреплена здесь.
    #[test]
    fn builds_a_web_address_launch_without_a_connection_string() {
        let connection = V8Connection::from_connection_string("File=/tmp/ib");
        let args = build_launch_args(
            LaunchClientMode::Thin,
            &ClientAddress::Web("http://localhost/base".to_owned()),
            &connection,
            &[],
            &LaunchOptions::default(),
        );

        assert_eq!(
            args,
            vec![
                "ENTERPRISE".to_owned(),
                "/DisableStartupDialogs".to_owned(),
                "/WS".to_owned(),
                "http://localhost/base".to_owned(),
            ]
        );
    }

    /// Реквизиты при клиентском адресе идут, только если так решено: тогда `/N` и `/P`
    /// следуют за `/WS`, иначе в командной строке их нет вовсе.
    #[test]
    fn a_web_address_carries_the_credentials_only_when_decided() {
        let mut connection = V8Connection::from_connection_string("Srvr=h:1541;Ref=demo");
        connection.user = Some("Admin".to_owned());
        connection.password = Some("s3cret".to_owned());
        let args = |address: ClientAddress| {
            build_launch_args(
                LaunchClientMode::Thin,
                &address,
                &connection,
                &[],
                &LaunchOptions::default(),
            )
        };

        let with = args(ClientAddress::Web("http://localhost/base".to_owned()));
        let without = args(ClientAddress::WebWithoutCredentials(
            "http://localhost/base".to_owned(),
        ));

        assert_eq!(
            &with[2..],
            [
                "/WS",
                "http://localhost/base",
                "/N",
                "Admin",
                "/P",
                "s3cret"
            ]
        );
        assert_eq!(&without[2..], ["/WS", "http://localhost/base"]);
        assert!(!with.iter().chain(&without).any(|arg| arg == "/S"));
    }

    /// The 1C command-line parser as measured: values are separated by any
    /// whitespace, a quoted value reads `""` as one quote and ends at a lone
    /// quote. On
    /// macOS and Linux the platform first rebuilds a command line from argv,
    /// framing only an argument that contains an ASCII space; Windows parses
    /// the raw command line it was given.
    fn platform_parameters(encoded: &str) -> Vec<String> {
        let line = if !cfg!(windows) && encoded.contains(' ') {
            format!("\"{encoded}\"")
        } else {
            encoded.to_owned()
        };
        let mut values = Vec::new();
        let mut chars = line.chars().peekable();
        while chars.peek().is_some() {
            while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }
            let mut value = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                loop {
                    match chars.next() {
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            value.push('"');
                        }
                        Some('"') | None => break,
                        Some(ch) => value.push(ch),
                    }
                }
            } else {
                while let Some(ch) = chars.peek().copied() {
                    if ch.is_whitespace() {
                        break;
                    }
                    value.push(ch);
                    chars.next();
                }
            }
            values.push(value);
        }
        values
    }

    #[test]
    fn typed_application_parameter_reaches_the_client_as_exactly_one_value() {
        for input in [
            "",
            "plain",
            "Юника пробелы = тест",
            "Юника \"кавычки\" пробелы = тест",
            "\"",
            "\"\"",
            "\"abc",
            "abc\"",
            "abc\"def",
            "\"abc def",
            "x\" /Execute \"evil.epf",
            "x\"\t/Execute\t\"evil.epf",
            "/Execute",
            "/Execute evil.epf",
            "plain\tvalue",
            "a\t/Execute\tevil.epf",
            "\"abc\tdef",
            "\"abc\ndef",
            "line\n/Execute evil.epf",
            "--execute=ignored",
        ] {
            let command = build_launch_command(
                LaunchClientMode::Thin,
                &ClientAddress::Connection,
                &V8Connection::from_connection_string("File=/tmp/ib"),
                &[],
                &LaunchOptions {
                    c: Some(input.to_owned()),
                    execute: Some("/tmp/unchanged \"processor\".epf".to_owned()),
                    raw_args: vec!["/custom=\"unchanged\"".to_owned()],
                    ..Default::default()
                },
            );
            let args = &command.args;
            let position = args
                .iter()
                .position(|arg| arg == "/C")
                .expect("typed parameter");
            assert_eq!(args[position + 1], encode_launch_parameter(input));
            assert_eq!(command.verbatim_args, vec![position + 1]);
            assert_eq!(
                platform_parameters(&args[position + 1]),
                vec![input.to_owned()],
                "{input:?} must reach 1C as exactly one /C value"
            );
            assert!(args.contains(&"/custom=\"unchanged\"".to_owned()));
            assert!(args
                .windows(2)
                .any(|pair| pair == ["/Execute", "/tmp/unchanged \"processor\".epf"]));
        }
        let omitted = build_launch_command(
            LaunchClientMode::Thin,
            &ClientAddress::Connection,
            &V8Connection::from_connection_string("File=/tmp/ib"),
            &[],
            &LaunchOptions::default(),
        );
        assert!(!omitted.args.iter().any(|arg| arg == "/C"));
        assert!(omitted.verbatim_args.is_empty());
    }

    /// The verbatim index comes from where the `/C` value was emitted, not from a
    /// search: a user name or password spelled `/C` must not take its place.
    #[test]
    fn credentials_spelled_like_the_launch_key_do_not_take_the_verbatim_slot() {
        let mut connection = V8Connection::from_connection_string("File=/tmp/ib");
        connection.user = Some("/C".to_owned());
        connection.password = Some("/C".to_owned());
        let command = build_launch_command(
            LaunchClientMode::Thin,
            &ClientAddress::Connection,
            &connection,
            &[],
            &LaunchOptions {
                c: Some("x\" /Execute \"evil.epf".to_owned()),
                ..Default::default()
            },
        );

        assert_eq!(command.verbatim_args.len(), 1);
        let value = command.verbatim_args[0];
        assert_eq!(command.args[value - 1], "/C");
        assert_eq!(
            command.args[value],
            encode_launch_parameter("x\" /Execute \"evil.epf")
        );
        assert!(command.args.windows(2).any(|pair| pair == ["/N", "/C"]));
        assert!(command.args.windows(2).any(|pair| pair == ["/P", "/C"]));
    }

    /// The enterprise DSL hands the verbatim index to the process runner.
    #[test]
    fn enterprise_dsl_passes_the_verbatim_index_to_the_runner() {
        struct Recorder(std::sync::Mutex<Option<ProcessRequest>>);
        impl ProcessRunner for Recorder {
            fn run_with_policy(
                &self,
                request: &ProcessRequest,
                policy: &ProcessExecutionPolicy,
            ) -> Result<ProcessResult, ProcessError> {
                policy.mark_started_for_test();
                *self.0.lock().expect("request lock") = Some(request.clone());
                Ok(ProcessResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    interruption: None,
                })
            }

            fn spawn(
                &self,
                _request: &ProcessRequest,
                _work: &WorkGiven,
            ) -> Result<SpawnResult, ProcessError> {
                panic!("the enterprise DSL runs the client, it never spawns it")
            }
        }

        let dir = tempdir().expect("tempdir");
        let runner = Recorder(std::sync::Mutex::new(None));
        let dsl = EnterpriseDsl::new(
            dir.path().join("1cv8c"),
            V8Connection::from_connection_string("File=/tmp/ib"),
            ClientAddress::Connection,
            Vec::new(),
            LaunchClientMode::Thin,
            &runner as &dyn ProcessRunner,
            dir.path().join("platform.log"),
            ProcessExecutionPolicy::default(),
        );
        dsl.run_launch(&LaunchOptions {
            c: Some("RunUnitTests=/tmp/test.json".to_owned()),
            ..LaunchOptions::default()
        })
        .expect("launch runs");

        let request = runner.0.lock().expect("request lock").take().expect("ran");
        assert_eq!(request.windows_verbatim_args.len(), 1);
        let value = request.windows_verbatim_args[0];
        assert_eq!(request.args[value - 1], "/C");
        assert_eq!(
            request.args[value],
            encode_launch_parameter("RunUnitTests=/tmp/test.json")
        );
    }

    /// The measured macOS 8.3.27 table: the argv that delivered each value.
    #[test]
    #[cfg(not(windows))]
    fn typed_application_parameter_matches_the_measured_macos_argv() {
        for (input, measured) in [
            ("", "\"\""),
            (
                "Юника \"кавычки\" пробелы = тест",
                "Юника \"\"кавычки\"\" пробелы = тест",
            ),
            ("\"", "\"\"\"\""),
            ("\"\"", "\"\"\"\"\"\""),
            ("\"abc", "\"\"\"abc\""),
            ("abc\"def", "\"abc\"\"def\""),
            ("abc\"", "\"abc\"\"\""),
            ("\"abc def", "\"\"abc def"),
            ("\"abc\tdef", "\"\"\"abc\tdef\""),
            ("\"abc\ndef", "\"\"\"abc\ndef\""),
        ] {
            assert_eq!(encode_launch_parameter(input), measured, "{input:?}");
        }
    }

    #[test]
    fn builds_expected_run_unit_tests_arguments() {
        let args = build_launch_args(
            LaunchClientMode::Thin,
            &ClientAddress::Connection,
            &V8Connection::from_connection_string("File=/tmp/ib"),
            &["/TESTMANAGER".to_owned()],
            &LaunchOptions {
                c: Some("RunUnitTests=/tmp/path with space/тест config.json".to_owned()),
                internal_out: Some("/tmp/platform.log".to_owned()),
                ..LaunchOptions::default()
            },
        );

        assert_eq!(args[0], "ENTERPRISE");
        assert_eq!(args[1], "/DisableStartupDialogs");
        assert!(args.iter().any(|arg| arg == "/TESTMANAGER"));
        assert!(args.windows(2).any(|pair| pair
            == [
                "/C".to_owned(),
                encode_launch_parameter("RunUnitTests=/tmp/path with space/тест config.json"),
            ]));
        assert!(!args
            .iter()
            .any(|arg| arg == "/C\"RunUnitTests=/tmp/path with space/тест config.json\""));
        assert!(args.iter().any(|arg| arg == "/Out"));
    }

    #[test]
    fn builds_expected_vanessa_arguments() {
        let args = build_launch_args(
            LaunchClientMode::Thin,
            &ClientAddress::Connection,
            &V8Connection::from_connection_string("File=/tmp/ib"),
            &["/TESTMANAGER".to_owned()],
            &LaunchOptions {
                execute: Some("/tmp/va/vanessa automation.epf".to_owned()),
                c: Some("StartFeaturePlayer;VAParams=/tmp/va/va-params.json".to_owned()),
                internal_out: Some("/tmp/platform.log".to_owned()),
                ..LaunchOptions::default()
            },
        );

        assert_eq!(args[0], "ENTERPRISE");
        assert!(args.iter().any(|arg| arg == "/Execute"));
        assert!(args
            .iter()
            .any(|arg| arg == "/tmp/va/vanessa automation.epf"));
        assert!(args.iter().any(|arg| arg == "/TESTMANAGER"));
        assert!(args.windows(2).any(|pair| pair
            == [
                "/C".to_owned(),
                encode_launch_parameter("StartFeaturePlayer;VAParams=/tmp/va/va-params.json"),
            ]));
        assert!(!args
            .iter()
            .any(|arg| arg == "/C\"StartFeaturePlayer;VAParams=/tmp/va/va-params.json\""));
    }

    #[test]
    fn ordinary_mode_adds_run_mode_and_filters_reserved_raw_keys() {
        let args = build_launch_args(
            LaunchClientMode::Ordinary,
            &ClientAddress::Connection,
            &V8Connection::from_connection_string("File=/tmp/ib"),
            &[
                "/TESTMANAGER".to_owned(),
                "/DisableStartupDialogs".to_owned(),
            ],
            &LaunchOptions {
                use_privileged_mode: true,
                raw_args: vec![
                    "/RunModeOrdinaryApplication".to_owned(),
                    "/Out".to_owned(),
                    "user.log".to_owned(),
                    "/C".to_owned(),
                    "ignored".to_owned(),
                    "/C\"attached\"".to_owned(),
                    "/C spaced".to_owned(),
                    "/C=assigned".to_owned(),
                    "/Execute:tool.epf".to_owned(),
                    "/Out=raw.log".to_owned(),
                    "/WA-".to_owned(),
                ],
                out: Some("launch.log".to_owned()),
                ..LaunchOptions::default()
            },
        );

        assert!(args.iter().any(|arg| arg == "/RunModeOrdinaryApplication"));
        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_str() == "/DisableStartupDialogs")
                .count(),
            1
        );
        assert!(args.iter().any(|arg| arg == "/UsePrivilegedMode"));
        assert!(args.iter().any(|arg| arg == "/WA-"));
        assert!(!args.iter().any(|arg| arg == "ignored"));
        assert!(!args.iter().any(|arg| arg == "/C\"attached\""));
        assert!(!args.iter().any(|arg| arg == "/C spaced"));
        assert!(!args.iter().any(|arg| arg == "/C=assigned"));
        assert!(!args.iter().any(|arg| arg == "/Execute:tool.epf"));
        assert!(!args.iter().any(|arg| arg == "/Out=raw.log"));
        assert!(args.ends_with(&["/Out".to_owned(), "launch.log".to_owned()]));
    }

    #[test]
    fn internal_out_has_priority_over_user_out() {
        let args = build_launch_args(
            LaunchClientMode::Thin,
            &ClientAddress::Connection,
            &V8Connection::from_connection_string("File=/tmp/ib"),
            &[],
            &LaunchOptions {
                out: Some("user.log".to_owned()),
                internal_out: Some("internal.log".to_owned()),
                ..LaunchOptions::default()
            },
        );

        assert!(args.ends_with(&["/Out".to_owned(), "internal.log".to_owned()]));
        assert!(!args.iter().any(|arg| arg == "user.log"));
    }

    #[test]
    fn enterprise_dsl_applies_internal_out_to_launch() {
        let dir = tempdir().expect("tempdir");
        let runner = ProcessExecutor;
        let dsl = EnterpriseDsl::new(
            dir.path().join("1cv8c"),
            V8Connection::from_connection_string("File=/tmp/ib"),
            ClientAddress::Connection,
            vec!["/TESTMANAGER".to_owned()],
            LaunchClientMode::Thin,
            &runner as &dyn ProcessRunner,
            dir.path().join("platform.log"),
            crate::platform::process::ProcessExecutionPolicy::default(),
        );

        let args = dsl
            .build_command(&LaunchOptions {
                c: Some("RunUnitTests=/tmp/test.json".to_owned()),
                out: Some("user.log".to_owned()),
                ..LaunchOptions::default()
            })
            .args;

        assert!(args.ends_with(&[
            "/Out".to_owned(),
            dir.path().join("platform.log").display().to_string()
        ]));
        assert!(!args.iter().any(|arg| arg == "user.log"));
    }
}
