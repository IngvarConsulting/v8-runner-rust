use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::domain::runner::LaunchClientModeRequest;
use crate::domain::runner::{launch_key_alias_matches, LaunchOptions};
use crate::platform::connection::V8Connection;
use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessInterruptionSafety, ProcessRequest, ProcessRunner,
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
        additional_launch_keys: Vec<String>,
        client_mode: LaunchClientMode,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
        timeout: Duration,
    ) -> Self {
        Self {
            binary,
            connection,
            additional_launch_keys,
            client_mode,
            runner,
            log_file,
            execution_policy: ProcessExecutionPolicy::new(
                Some(timeout),
                CancellationToken::new(),
                ProcessInterruptionSafety::GracefulThenKill,
            ),
        }
    }

    /// Overrides the shared execution policy for launching Enterprise.
    pub fn with_execution_policy(mut self, execution_policy: ProcessExecutionPolicy) -> Self {
        self.execution_policy = execution_policy;
        self
    }

    pub fn run_launch(
        &self,
        launch: &LaunchOptions,
    ) -> Result<PlatformCommandResult, EnterpriseError> {
        let args = self.build_args(launch);
        let process = self
            .runner
            .run_with_policy(
                &ProcessRequest {
                    program: self.binary.clone(),
                    args,
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
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

    fn build_args(&self, launch: &LaunchOptions) -> Vec<String> {
        let mut launch = launch.clone();
        launch.internal_out = Some(self.log_file.display().to_string());
        build_launch_args(
            self.client_mode,
            LaunchAddress::Connection(&self.connection),
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

/// Чем клиент открывает базу в командной строке.
///
/// Административный адрес несёт реквизиты базы рядом с собой; клиентский — не всегда:
/// у автономной цели `infobase.user`/`password` принадлежат SSH-шлюзу, и клиенту их
/// отдавать нельзя. Поэтому реквизиты у веб-адреса — отдельное, необязательное поле.
#[derive(Debug, Clone, Copy)]
pub enum LaunchAddress<'a> {
    /// `infobase.connection` вместе с `/N` и `/P`.
    Connection(&'a V8Connection),
    /// `infobase.web.url` как ws-соединение; реквизиты прилагаются, только если они
    /// действительно реквизиты базы.
    Web {
        url: &'a str,
        credentials: Option<&'a V8Connection>,
    },
}

impl LaunchAddress<'_> {
    fn args(&self) -> Vec<String> {
        match *self {
            Self::Connection(connection) => connection.args(),
            Self::Web { url, credentials } => {
                let mut args = vec!["/WS".to_owned(), url.to_string()];
                if let Some(connection) = credentials {
                    args.extend(connection.credential_args());
                }
                args
            }
        }
    }
}

pub fn build_launch_args(
    mode: LaunchClientMode,
    address: LaunchAddress<'_>,
    additional_launch_keys: &[String],
    launch: &LaunchOptions,
) -> Vec<String> {
    let mut args = vec![match mode {
        LaunchClientMode::Designer => "DESIGNER",
        LaunchClientMode::Thin | LaunchClientMode::Thick | LaunchClientMode::Ordinary => {
            "ENTERPRISE"
        }
    }
    .to_owned()];
    args.push("/DisableStartupDialogs".to_owned());
    args.extend(address.args());
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
    if let Some(c) = &launch.c {
        args.push("/C".to_owned());
        args.push(c.clone());
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
    args
}

/// Replacement written in place of every credential value exposed by a launch preview.
pub const MASKED_LAUNCH_VALUE: &str = "***";

/// Rewrite composed launch arguments so no credential value survives into a preview.
///
/// Four independent rules apply, because a secret can reach argv four ways: as the
/// value of the `/P` key the runner itself appends, as a `Pwd=` segment inside a raw
/// connection string, as a literal the caller glued to a key the runner does not
/// recognise, and as userinfo inside the client address after `/WS`. `secrets` carries
/// values known to be confidential, masked wherever they appear.
///
/// Userinfo прячется только у значения `/WS`, а не у каждого аргумента подряд: без схемы
/// на адрес похож и путь вида `C:\dir@host`, и сплошная маскировка портила бы аргументы,
/// никаких секретов не содержащие.
pub fn mask_launch_args(args: &[String], secrets: &[&str]) -> Vec<String> {
    let mut masked = Vec::with_capacity(args.len());
    let mut mask_detached_value = false;
    let mut mask_next_address = false;
    for arg in args {
        if mask_detached_value {
            mask_detached_value = false;
            masked.push(MASKED_LAUNCH_VALUE.to_owned());
            continue;
        }
        if mask_next_address {
            mask_next_address = false;
            masked.push(mask_url_userinfo(arg));
            continue;
        }
        if is_client_address_key(arg) {
            mask_next_address = true;
            masked.push(arg.clone());
            continue;
        }
        match password_key_value_start(arg) {
            Some(start) if start == arg.len() => {
                mask_detached_value = true;
                masked.push(arg.clone());
            }
            Some(start) => masked.push(format!("{}{MASKED_LAUNCH_VALUE}", &arg[..start])),
            None => masked.push(mask_literal_secrets(
                &mask_connection_string_password(arg),
                secrets,
            )),
        }
    }
    masked
}

/// Прячет пароль из userinfo адреса: `http://alice:pass@host/base` → `http://alice:***@host/base`.
///
/// Маскируется только то, что после `:`. Голое имя пользователя секретом не является, а
/// спрятать его целиком значит сделать адрес неузнаваемым — а он нужен человеку, чтобы
/// понять, куда именно раннер собрался.
pub fn mask_url_userinfo(value: &str) -> String {
    // Адрес приходит из `infobase.web.url`, а это поле не валидируется вовсе, поэтому
    // схемы может не быть. Начало authority ищем во всех трёх видах: со схемой,
    // схемо-относительный и голый.
    let authority_start = match value.find("://") {
        Some(scheme_end) => scheme_end + "://".len(),
        None if value.starts_with("//") => "//".len(),
        None => 0,
    };
    let authority_end = value[authority_start..]
        .find(['/', '?', '#'])
        .map_or(value.len(), |at| authority_start + at);
    let authority = &value[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return value.to_owned();
    };
    let Some(colon) = authority[..at].find(':') else {
        return value.to_owned();
    };
    format!(
        "{}{}:{MASKED_LAUNCH_VALUE}{}",
        &value[..authority_start],
        &authority[..colon],
        &value[authority_start + at..]
    )
}

/// Ключ, за которым идёт клиентский адрес: его значение маскируется как адрес, а не целиком.
fn is_client_address_key(arg: &str) -> bool {
    arg.strip_prefix('/')
        .or_else(|| arg.strip_prefix('-'))
        .is_some_and(|rest| rest.eq_ignore_ascii_case("ws"))
}

/// Byte offset at which a `/P` key's value starts, or `arg.len()` when the value is detached.
fn password_key_value_start(arg: &str) -> Option<usize> {
    let rest = arg.strip_prefix('/').or_else(|| arg.strip_prefix('-'))?;
    let split_at = rest.char_indices().nth(1).map_or(rest.len(), |(at, _)| at);
    let (head, tail) = rest.split_at(split_at);
    if !head.eq_ignore_ascii_case("p") {
        return None;
    }
    if tail.is_empty() {
        return Some(arg.len());
    }
    // A glued `/Psecret` is indistinguishable from an unrelated key such as `/Proxy`,
    // so only an explicit separator is read as a value here; the literal rule covers
    // the glued form for secrets whose value is known.
    let separator = tail.chars().next()?;
    if matches!(separator, ' ' | '=' | ':' | '"') {
        return Some(arg.len() - tail.len() + separator.len_utf8());
    }
    None
}

/// Mask the `Pwd=` segment of a 1C connection string, keeping every other segment readable.
fn mask_connection_string_password(arg: &str) -> String {
    let lowered = arg.to_ascii_lowercase();
    let mut masked = String::with_capacity(arg.len());
    let mut cursor = 0;
    while let Some(found) = lowered[cursor..].find("pwd=") {
        let key_start = cursor + found;
        let value_start = key_start + "pwd=".len();
        let preceded_by_boundary = key_start == 0
            || matches!(
                lowered[..key_start].chars().next_back(),
                Some(';' | ' ' | '\'' | '"')
            );
        if !preceded_by_boundary {
            masked.push_str(&arg[cursor..value_start]);
            cursor = value_start;
            continue;
        }
        let value_end = arg[value_start..]
            .find(';')
            .map_or(arg.len(), |at| value_start + at);
        masked.push_str(&arg[cursor..value_start]);
        masked.push_str(MASKED_LAUNCH_VALUE);
        cursor = value_end;
    }
    masked.push_str(&arg[cursor..]);
    masked
}

/// Mask known confidential literals wherever they appear inside one argument.
fn mask_literal_secrets(arg: &str, secrets: &[&str]) -> String {
    let mut masked = arg.to_owned();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        masked = masked.replace(secret, MASKED_LAUNCH_VALUE);
    }
    masked
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
        build_launch_args, mask_launch_args, mask_url_userinfo, normalize_launch_payload_path,
        EnterpriseDsl, LaunchAddress, LaunchClientMode,
    };
    use crate::domain::runner::LaunchOptions;
    use crate::platform::connection::V8Connection;
    use crate::platform::process::{ProcessExecutor, ProcessRunner};
    use std::path::Path;
    use std::time::Duration;
    use tempfile::tempdir;

    fn masked(args: &[&str], secrets: &[&str]) -> Vec<String> {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        mask_launch_args(&owned, secrets)
    }

    #[test]
    fn masks_the_detached_password_value_and_keeps_the_user_readable() {
        let args = masked(
            &["ENTERPRISE", "/N", "Администратор", "/P", "s3cret"],
            &["s3cret"],
        );
        assert_eq!(args, vec!["ENTERPRISE", "/N", "Администратор", "/P", "***"]);
    }

    #[test]
    fn masks_every_attached_password_separator_form() {
        for arg in ["/P=s3cret", "/P:s3cret", "-p=s3cret", "/P s3cret"] {
            let args = masked(&[arg], &[]);
            assert!(
                !args[0].contains("s3cret") && args[0].ends_with("***"),
                "{arg} -> {}",
                args[0]
            );
        }
    }

    #[test]
    fn keeps_unrelated_keys_that_merely_start_with_p() {
        let args = masked(&["/Proxy", "/PublishWSOnDemand"], &[]);
        assert_eq!(args, vec!["/Proxy", "/PublishWSOnDemand"]);
    }

    #[test]
    fn masks_only_the_password_segment_of_a_connection_string() {
        let args = masked(
            &[
                "/IBConnectionString",
                "Srvr=\"srv:1541\";Ref=\"ut\";Usr=Админ;Pwd=s3cret;",
            ],
            &[],
        );
        assert_eq!(args[1], "Srvr=\"srv:1541\";Ref=\"ut\";Usr=Админ;Pwd=***;");
    }

    #[test]
    fn masks_a_known_secret_glued_to_an_unrecognised_key() {
        let args = masked(&["/Pses3cret"], &["s3cret"]);
        assert_eq!(args, vec!["/Pse***"]);
    }

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
            LaunchAddress::Web {
                url: "http://localhost/base",
                credentials: Some(&connection),
            },
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

    /// У автономной цели `infobase.user` и `infobase.password` — данные SSH-шлюза, а не
    /// базы, поэтому реквизиты к адресу не прилагаются.
    #[test]
    fn a_web_address_without_credentials_carries_no_user_keys() {
        let mut connection = V8Connection::from_connection_string("File=/tmp/ib");
        connection.user = Some("gate".to_owned());
        connection.password = Some("gate-secret".to_owned());

        let with_credentials = build_launch_args(
            LaunchClientMode::Thin,
            LaunchAddress::Web {
                url: "http://localhost/base",
                credentials: Some(&connection),
            },
            &[],
            &LaunchOptions::default(),
        );
        let without = build_launch_args(
            LaunchClientMode::Thin,
            LaunchAddress::Web {
                url: "http://localhost/base",
                credentials: None,
            },
            &[],
            &LaunchOptions::default(),
        );

        assert!(with_credentials.contains(&"/N".to_owned()));
        assert!(with_credentials.contains(&"gate-secret".to_owned()));
        assert!(!without.contains(&"/N".to_owned()));
        assert!(!without.contains(&"/P".to_owned()));
        assert!(!without.iter().any(|arg| arg.contains("gate-secret")));
    }

    /// Разбор адреса написан руками, поэтому таблица форм: со схемой и без неё, IPv6,
    /// `@` в пути и запросе, пустой пароль, голое имя пользователя, повторное применение.
    #[test]
    fn mask_url_userinfo_hides_the_password_in_every_shape_of_address() {
        for (value, expected) in [
            (
                "http://alice:s3cret@host/base",
                "http://alice:***@host/base",
            ),
            (
                "https://alice:s3cret@host:443/b?x=1#f",
                "https://alice:***@host:443/b?x=1#f",
            ),
            // Схемы может не быть вовсе: поле не валидируется.
            ("//alice:s3cret@host/base", "//alice:***@host/base"),
            ("alice:s3cret@host/base", "alice:***@host/base"),
            // Пароль с разделителями внутри: маскируется от первого `:` до последней `@`.
            ("http://alice:p@ss:word@host/b", "http://alice:***@host/b"),
            (
                "http://alice:s3cret@[2001:db8::1]:8080/b",
                "http://alice:***@[2001:db8::1]:8080/b",
            ),
            // Прятать нечего.
            ("http://alice@host/base", "http://alice@host/base"),
            ("http://host/base", "http://host/base"),
            (
                "http://[2001:db8::1]:8080/base",
                "http://[2001:db8::1]:8080/base",
            ),
            ("http://host/path@with-at", "http://host/path@with-at"),
            ("http://host/base?q=a@b", "http://host/base?q=a@b"),
            // Уже замаскированное второй раз не портится.
            ("http://alice:***@host/base", "http://alice:***@host/base"),
        ] {
            assert_eq!(mask_url_userinfo(value), expected, "вход: {value}");
        }
    }

    /// Маскируется значение `/WS`, а не всё, что похоже на адрес: путь с `@` и строка
    /// подключения остаются читаемыми.
    #[test]
    fn only_the_client_address_is_masked_as_an_address() {
        let masked = mask_launch_args(
            &[
                "/WS".to_owned(),
                "http://alice:s3cret@host/base".to_owned(),
                "/C".to_owned(),
                "C:\\dir@host".to_owned(),
                "/IBConnectionString".to_owned(),
                "Srvr=host;Ref=base".to_owned(),
            ],
            &[],
        );

        assert_eq!(masked[1], "http://alice:***@host/base");
        assert_eq!(
            masked[3], "C:\\dir@host",
            "путь не адрес и портиться не должен"
        );
        assert_eq!(masked[5], "Srvr=host;Ref=base");
    }

    #[test]
    fn builds_expected_run_unit_tests_arguments() {
        let args = build_launch_args(
            LaunchClientMode::Thin,
            LaunchAddress::Connection(&V8Connection::from_connection_string("File=/tmp/ib")),
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
                "RunUnitTests=/tmp/path with space/тест config.json".to_owned(),
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
            LaunchAddress::Connection(&V8Connection::from_connection_string("File=/tmp/ib")),
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
                "StartFeaturePlayer;VAParams=/tmp/va/va-params.json".to_owned(),
            ]));
        assert!(!args
            .iter()
            .any(|arg| arg == "/C\"StartFeaturePlayer;VAParams=/tmp/va/va-params.json\""));
    }

    #[test]
    fn ordinary_mode_adds_run_mode_and_filters_reserved_raw_keys() {
        let args = build_launch_args(
            LaunchClientMode::Ordinary,
            LaunchAddress::Connection(&V8Connection::from_connection_string("File=/tmp/ib")),
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
            LaunchAddress::Connection(&V8Connection::from_connection_string("File=/tmp/ib")),
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
            vec!["/TESTMANAGER".to_owned()],
            LaunchClientMode::Thin,
            &runner as &dyn ProcessRunner,
            dir.path().join("platform.log"),
            Duration::from_secs(5),
        );

        let args = dsl.build_args(&LaunchOptions {
            c: Some("RunUnitTests=/tmp/test.json".to_owned()),
            out: Some("user.log".to_owned()),
            ..LaunchOptions::default()
        });

        assert!(args.ends_with(&[
            "/Out".to_owned(),
            dir.path().join("platform.log").display().to_string()
        ]));
        assert!(!args.iter().any(|arg| arg == "user.log"));
    }
}
