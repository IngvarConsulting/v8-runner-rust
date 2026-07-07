use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::loader::load_config;
use crate::config::schema::{local_config_schema_url, main_config_schema_url};
use crate::domain::bootstrap::BootstrapResult;
use crate::platform::connection::V8Connection;
use crate::support::error::AppError;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::dump_config;
use crate::use_cases::request::{DumpModeRequest, DumpRequest};
use crate::use_cases::result::{UseCaseError, UseCaseFailure, UseCaseResult};

const CONFIG_FILE_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";
const GITIGNORE_FILE_NAME: &str = ".gitignore";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapRequest {
    pub project_dir: PathBuf,
    pub connection: String,
    pub platform_version: String,
    pub platform_path: Option<PathBuf>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub source_dir: PathBuf,
    pub force: bool,
}

pub fn execute(
    context: &ExecutionContext,
    request: &BootstrapRequest,
) -> UseCaseResult<BootstrapResult> {
    let started = Instant::now();
    if let Err(error) = reject_embedded_credentials(&request.connection) {
        return Err(UseCaseFailure::without_payload(error));
    }

    let project_dir = match resolve_project_dir(&request.project_dir) {
        Ok(project_dir) => project_dir,
        Err(error) => return Err(UseCaseFailure::without_payload(error)),
    };
    let paths = BootstrapPaths::new(&project_dir, &request.source_dir);
    if let Err(error) = preflight_targets(&paths, request.force) {
        return Err(UseCaseFailure::without_payload(error));
    }

    if let Err(error) = write_bootstrap_files(&paths, request) {
        return Err(UseCaseFailure::without_payload(error));
    }

    let config = match load_config(Some(&paths.config_path.display().to_string()), None) {
        Ok(config) => config,
        Err(error) => {
            return Err(UseCaseFailure::with_payload(
                AppError::from(error),
                bootstrap_result(
                    started,
                    &paths,
                    false,
                    Vec::new(),
                    Some("config load failed"),
                ),
            ));
        }
    };
    let dump_request = DumpRequest {
        mode: DumpModeRequest::Full,
        source_set: Some("main".to_owned()),
        extension: None,
        objects: Vec::new(),
    };
    match dump_config::execute(context, &config, &dump_request) {
        Ok(dump) => Ok(bootstrap_result(
            started,
            &paths,
            true,
            Vec::new(),
            dump.message.as_deref(),
        )),
        Err(failure) => {
            let error = failure.error;
            let message = redact_message(error.message(), request);
            let redacted_error = UseCaseError::new(error.kind(), message.clone());
            let payload_message = failure
                .payload
                .as_ref()
                .and_then(|dump| dump.message.as_deref())
                .map(|value| redact_message(value, request))
                .or(Some(message));
            let payload = bootstrap_result(started, &paths, false, Vec::new(), payload_message);
            Err(UseCaseFailure::with_payload(redacted_error, payload))
        }
    }
}

#[derive(Debug, Clone)]
struct BootstrapPaths {
    config_path: PathBuf,
    local_config_path: PathBuf,
    gitignore_path: PathBuf,
    source_dir: PathBuf,
}

impl BootstrapPaths {
    fn new(project_dir: &Path, source_dir: &Path) -> Self {
        Self {
            config_path: project_dir.join(CONFIG_FILE_NAME),
            local_config_path: project_dir.join(LOCAL_CONFIG_FILE_NAME),
            gitignore_path: project_dir.join(GITIGNORE_FILE_NAME),
            source_dir: if source_dir.is_absolute() {
                source_dir.to_path_buf()
            } else {
                project_dir.join(source_dir)
            },
        }
    }
}

fn resolve_project_dir(path: &Path) -> Result<PathBuf, AppError> {
    if path.exists() && !path.is_dir() {
        return Err(AppError::Validation(format!(
            "project directory is not a directory: {}",
            path.display()
        )));
    }
    std::fs::create_dir_all(path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to create project directory '{}': {error}",
            path.display()
        ))
    })?;
    std::fs::canonicalize(path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to resolve project directory '{}': {error}",
            path.display()
        ))
    })
}

fn preflight_targets(paths: &BootstrapPaths, force: bool) -> Result<(), AppError> {
    if force {
        return Ok(());
    }
    for path in [
        &paths.config_path,
        &paths.local_config_path,
        &paths.source_dir,
    ] {
        if path.exists() {
            return Err(AppError::Validation(format!(
                "bootstrap target already exists: {} (use --force to overwrite)",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_bootstrap_files(
    paths: &BootstrapPaths,
    request: &BootstrapRequest,
) -> Result<(), AppError> {
    std::fs::create_dir_all(paths.config_path.parent().unwrap_or(Path::new("."))).map_err(
        |error| {
            AppError::Runtime(format!(
                "failed to create project config directory: {error}"
            ))
        },
    )?;
    std::fs::write(&paths.config_path, render_main_config(paths, request)).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write config file '{}': {error}",
            paths.config_path.display()
        ))
    })?;
    ensure_gitignore(paths)?;
    std::fs::write(&paths.local_config_path, render_local_config(request)).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write local config file '{}': {error}",
            paths.local_config_path.display()
        ))
    })?;
    std::fs::create_dir_all(&paths.source_dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to create source directory '{}': {error}",
            paths.source_dir.display()
        ))
    })
}

fn render_main_config(paths: &BootstrapPaths, request: &BootstrapRequest) -> String {
    let source_path = relative_to_project(&paths.config_path, &paths.source_dir);
    let connection = render_bootstrap_connection(&request.connection);
    format!(
        "# yaml-language-server: $schema={}\n# Generated by v8-runner bootstrap\nworkPath: 'build'\nexecution_timeout: 300000\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: '{}'\nsource-set:\n  - name: 'main'\n    type: CONFIGURATION\n    path: '{}'\ntools:\n  platform:\n    version: '{}'\nbuild:\n  partialLoadThreshold: 20\n",
        main_config_schema_url(),
        escape_yaml(&connection),
        escape_yaml(&source_path),
        escape_yaml(&request.platform_version),
    )
}

fn render_local_config(request: &BootstrapRequest) -> String {
    let mut yaml = format!(
        "# yaml-language-server: $schema={}\n",
        local_config_schema_url()
    );
    if request.user.is_none() && request.password.is_none() && request.platform_path.is_none() {
        yaml.push_str("{}\n");
        return yaml;
    }
    if request.user.is_some() || request.password.is_some() {
        yaml.push_str("infobase:\n");
        if let Some(user) = &request.user {
            yaml.push_str(&format!("  user: '{}'\n", escape_yaml(user)));
        }
        if let Some(password) = &request.password {
            yaml.push_str(&format!("  password: '{}'\n", escape_yaml(password)));
        }
    }
    if let Some(platform_path) = &request.platform_path {
        yaml.push_str("tools:\n");
        yaml.push_str("  platform:\n");
        yaml.push_str(&format!(
            "    path: '{}'\n",
            escape_yaml(&platform_path.display().to_string())
        ));
    }
    yaml
}

fn ensure_gitignore(paths: &BootstrapPaths) -> Result<(), AppError> {
    let pattern = LOCAL_CONFIG_FILE_NAME;
    if paths.gitignore_path.exists() {
        let mut content = std::fs::read_to_string(&paths.gitignore_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to read gitignore file '{}': {error}",
                paths.gitignore_path.display()
            ))
        })?;
        if gitignore_mentions_local_config(&content) {
            return Ok(());
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(pattern);
        content.push('\n');
        std::fs::write(&paths.gitignore_path, content).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write gitignore file '{}': {error}",
                paths.gitignore_path.display()
            ))
        })?;
        return Ok(());
    }
    std::fs::write(&paths.gitignore_path, format!("{pattern}\n")).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write gitignore file '{}': {error}",
            paths.gitignore_path.display()
        ))
    })
}

fn gitignore_mentions_local_config(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        !line.is_empty()
            && !line.starts_with('#')
            && !line.starts_with('!')
            && matches!(
                line,
                LOCAL_CONFIG_FILE_NAME | "/v8project.local.yaml" | "**/v8project.local.yaml"
            )
    })
}

fn relative_to_project(config_path: &Path, path: &Path) -> String {
    let project_dir = config_path.parent().unwrap_or(Path::new("."));
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn render_bootstrap_connection(connection: &str) -> String {
    if let Some(file_path) = simple_file_connection_path(connection) {
        return format!("/F \"{}\"", file_path.replace('"', "\\\""));
    }
    connection.to_owned()
}

fn simple_file_connection_path(connection: &str) -> Option<&str> {
    let mut parts = connection
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let first = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (key, value) = first.split_once('=')?;
    key.trim()
        .eq_ignore_ascii_case("file")
        .then_some(unquote_connection_value(value.trim()))
        .filter(|value| !value.is_empty())
}

fn unquote_connection_value(value: &str) -> &str {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(value);
    };
    inner
}

fn reject_embedded_credentials(connection: &str) -> Result<(), AppError> {
    if connection.split(';').any(|part| {
        let key = part
            .split_once('=')
            .map(|(key, _)| key.trim())
            .unwrap_or_default();
        key.eq_ignore_ascii_case("usr")
            || key.eq_ignore_ascii_case("user")
            || key.eq_ignore_ascii_case("pwd")
            || key.eq_ignore_ascii_case("password")
    }) {
        return Err(AppError::Validation(
            "bootstrap connection must not contain embedded credentials; use --user and --password"
                .to_owned(),
        ));
    }

    let args = V8Connection::from_connection_string(connection).args();
    if args.iter().any(|arg| is_embedded_auth_arg(arg)) {
        return Err(AppError::Validation(
            "bootstrap connection must not contain embedded credentials; use --user and --password"
                .to_owned(),
        ));
    }
    Ok(())
}

fn is_embedded_auth_arg(arg: &str) -> bool {
    let key = arg.split_once('=').map_or(arg, |(key, _)| key);
    matches!(key.to_ascii_lowercase().as_str(), "/n" | "-n" | "/p" | "-p")
}

fn bootstrap_result(
    started: Instant,
    paths: &BootstrapPaths,
    dumped: bool,
    warnings: Vec<String>,
    message: Option<impl Into<String>>,
) -> BootstrapResult {
    BootstrapResult {
        ok: dumped,
        path: paths.config_path.clone(),
        local_path: paths.local_config_path.clone(),
        gitignore_path: paths.gitignore_path.clone(),
        source_dir: paths.source_dir.clone(),
        dump_target_path: paths.source_dir.clone(),
        dumped,
        warnings,
        message: message.map(Into::into),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn redact_message(message: &str, request: &BootstrapRequest) -> String {
    let mut redacted = message.to_owned();
    for secret in [&request.user, &request.password].into_iter().flatten() {
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "***");
        }
    }
    redacted
}

fn escape_yaml(value: &str) -> String {
    value.replace('\'', "''")
}
