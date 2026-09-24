use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::loader::{load_config, load_planned_config, LoadedConfig, ProjectText};
use crate::config::model::{AppConfig, InfobaseSelector};
use crate::config::schema::{local_config_schema_url, main_config_schema_url};
use crate::domain::bootstrap::BootstrapResult;
use crate::domain::dump::DumpResult;
use crate::platform::connection::unquote_connection_value;
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
    /// Превью: проект называется, но не пишется.
    pub dry_run: bool,
}

/// План клона: проверки, пути, текст проекта и его настройки. План ничего не пишет:
/// адаптер берёт по этим настройкам замок `workPath`, и только под ним появляется первый
/// файл проекта — занятый каталог отказывает, пока проекта ещё нет.
///
/// `Debug` у плана нет намеренно: запрос, текст местного слоя и настройки несут пароль.
pub struct ClonePlan {
    started: Instant,
    request: BootstrapRequest,
    paths: BootstrapPaths,
    main: String,
    local: String,
    settings: LoadedConfig,
}

impl ClonePlan {
    /// Настройки будущего проекта — те же, что прочтутся с диска после записи.
    pub fn config(&self) -> &AppConfig {
        &self.settings.config
    }

    /// Превью: план называет проект, но ни замка, ни записи ему не нужно.
    pub fn is_preview(&self) -> bool {
        self.request.dry_run
    }

    fn text(&self) -> ProjectText<'_> {
        ProjectText {
            project: &self.main,
            local_overlay: &self.local,
        }
    }

    fn refused(&self, error: AppError, message: String) -> UseCaseFailure<BootstrapResult> {
        refused_before_dump(self.started, &self.paths, error, message)
    }
}

/// Отказ до выгрузки: платформа не запускалась, ответ называет пути проекта.
fn refused_before_dump(
    started: Instant,
    paths: &BootstrapPaths,
    error: AppError,
    message: String,
) -> UseCaseFailure<BootstrapResult> {
    UseCaseFailure::with_payload(
        error,
        bootstrap_result(
            started,
            paths,
            BootstrapOutcome {
                ok: false,
                dumped: false,
                provider_dispatched: false,
                message: Some(message),
            },
            Vec::new(),
        ),
    )
}

pub fn plan(request: BootstrapRequest) -> Result<ClonePlan, UseCaseFailure<BootstrapResult>> {
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

    let main = render_main_config(&paths, &request);
    let local = render_local_config(&request);
    let planned = planned_settings(
        &paths,
        ProjectText {
            project: &main,
            local_overlay: &local,
        },
    );
    match planned {
        Ok(settings) => Ok(ClonePlan {
            started,
            request,
            paths,
            main,
            local,
            settings,
        }),
        // Выгрузки не было, а значит и платформы: отказ настроек приходит раньше выбора
        // исполнителя.
        Err(error) => Err(refused_before_dump(
            started,
            &paths,
            error,
            "config load failed".to_owned(),
        )),
    }
}

/// Клон по плану; вызывающий держит замок `workPath` проекта, превью его не берёт.
///
/// Боевой прогон пишет проект, читает его настройки с диска и выгружает базу; превью
/// выгрузку только планирует. Выгрузку ведёт один и тот же `dump_config`.
pub fn execute(context: &ExecutionContext, plan: &ClonePlan) -> UseCaseResult<BootstrapResult> {
    let request = &plan.request;
    let written;
    let LoadedConfig { config, warnings } = if plan.is_preview() {
        &plan.settings
    } else {
        // Под замком файлы проекта проверяются заново: другой клон мог написать их между
        // планом и замком. Каталог исходников — нет: его мог завести сам замок, если он
        // совпадает с `workPath`. Отмена, пришедшая до записи, не оставляет полупроекта.
        refuse_existing(
            &[&plan.paths.config_path, &plan.paths.local_config_path],
            request.force,
        )
        .map_err(UseCaseFailure::without_payload)?;
        if context.cancellation().is_cancelled() {
            return Err(UseCaseFailure::without_payload(AppError::Cancelled(
                "clone cancelled before the project was written".to_owned(),
            )));
        }
        // Запись — единственная собственная запись команды, и превью её не делает. Отказ
        // на ней остаётся отказом записи: подменять его отказом настроек значило бы
        // назвать не ту причину.
        if let Err(error) = write_bootstrap_files(&plan.paths, &plan.text()) {
            return Err(UseCaseFailure::without_payload(error));
        }
        written = match written_settings(&plan.paths) {
            Ok(settings) => settings,
            Err(error) => return Err(plan.refused(error, "config load failed".to_owned())),
        };
        // Замок взят по настройкам плана; написанный проект обязан назвать тот же каталог.
        if written.config.work_path != plan.config().work_path {
            let error = AppError::Runtime(format!(
                "the written project names workPath '{}', but the plan locked '{}'",
                written.config.work_path.display(),
                plan.config().work_path.display()
            ));
            let message = error.to_string();
            return Err(plan.refused(error, message));
        }
        &written
    };

    let dump_request = DumpRequest {
        dry_run: request.dry_run,
        mode: DumpModeRequest::Full,
        source_set: Some("main".to_owned()),
        extension: None,
        objects: Vec::new(),
        discard_uncommitted: false,
    };
    match dump_config::execute(context, config, &dump_request) {
        Ok(dump) => Ok(bootstrap_result(
            plan.started,
            &plan.paths,
            outcome_of(&dump, dump.message.clone()),
            warnings.clone(),
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
            // Признак запуска берётся у выгрузки и здесь: отказ тоже знает, дошло ли дело
            // до платформы. Его отсутствие значит `false` — ответа не было вовсе.
            let provider_dispatched = failure
                .payload
                .as_ref()
                .is_some_and(|dump| dump.provider_dispatched);
            let payload = bootstrap_result(
                plan.started,
                &plan.paths,
                BootstrapOutcome {
                    ok: false,
                    dumped: false,
                    provider_dispatched,
                    message: payload_message,
                },
                warnings.clone(),
            );
            Err(UseCaseFailure::with_payload(redacted_error, payload))
        }
    }
}

/// Настройки проекта, который будет написан: тот же текст, разобранный в памяти.
///
/// Текст у плана и записи один и складывают его одни и те же рендеры, поэтому
/// расходиться нечему. Предупреждения загрузчика идут дальше в ответ: они одинаково
/// верны и для написанного проекта, и для запланированного.
fn planned_settings(
    paths: &BootstrapPaths,
    text: ProjectText<'_>,
) -> Result<LoadedConfig, AppError> {
    load_planned_config(&paths.config_path, text, &InfobaseSelector::Default)
        .map_err(AppError::from)
}

/// Настройки написанного проекта — с диска.
fn written_settings(paths: &BootstrapPaths) -> Result<LoadedConfig, AppError> {
    load_config(
        Some(&paths.config_path.display().to_string()),
        None,
        &InfobaseSelector::Default,
    )
    .map_err(AppError::from)
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

/// Каталог проекта разрешается, но не создаётся: боевой прогон заводит его замком
/// `workPath`, а превью не пишет вовсе. На существующем каталоге ответ дословно тот же,
/// что у `canonicalize`.
fn resolve_project_dir(path: &Path) -> Result<PathBuf, AppError> {
    if path.exists() && !path.is_dir() {
        return Err(AppError::Validation(format!(
            "project directory is not a directory: {}",
            path.display()
        )));
    }
    // Приставка `\\?\` снимается здесь же: боевой загрузчик снимает её со своего пути, и
    // без этого превью считало бы относительные пути от текстуально другого корня.
    crate::support::path::nearest_existing_canonical_path(path)
        .map(|canonical| crate::support::path::normalize_windows_verbatim_path(&canonical))
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to resolve project directory '{}': {error}",
                path.display()
            ))
        })
}

fn preflight_targets(paths: &BootstrapPaths, force: bool) -> Result<(), AppError> {
    refuse_existing(
        &[
            &paths.config_path,
            &paths.local_config_path,
            &paths.source_dir,
        ],
        force,
    )
}

fn refuse_existing(targets: &[&Path], force: bool) -> Result<(), AppError> {
    if force {
        return Ok(());
    }
    for path in targets {
        if path.exists() {
            return Err(AppError::Validation(format!(
                "clone target already exists: {} (use --force to overwrite)",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_bootstrap_files(paths: &BootstrapPaths, text: &ProjectText<'_>) -> Result<(), AppError> {
    let project_dir = paths.config_path.parent().unwrap_or(Path::new("."));
    // Каталог проекта создаётся не при разрешении пути: план ничего не пишет. Боевой
    // прогон заводит его уже замком `workPath` внутри проекта, здесь он лишь гарантирован.
    std::fs::create_dir_all(project_dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to create project directory '{}': {error}",
            project_dir.display()
        ))
    })?;
    std::fs::write(&paths.config_path, text.project).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write config file '{}': {error}",
            paths.config_path.display()
        ))
    })?;
    ensure_gitignore(paths)?;
    std::fs::write(&paths.local_config_path, text.local_overlay).map_err(|error| {
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
    format!(
        "# yaml-language-server: $schema={}\n# Generated by v8-runner clone\nworkPath: 'build'\nformat: DESIGNER\nsource-set:\n  - name: 'main'\n    type: CONFIGURATION\n    path: '{}'\ntools:\n  platform:\n    version: '{}'\npush:\n  partialLoadThreshold: 20\n",
        main_config_schema_url(),
        escape_yaml(&source_path),
        escape_yaml(&request.platform_version),
    )
}

/// Местный слой объявляет `origin` целиком: адрес, а с ним учётные данные — они не
/// коммитятся вместе с проектом.
fn render_local_config(request: &BootstrapRequest) -> String {
    let connection = render_bootstrap_connection(&request.connection);
    let mut yaml = format!(
        "# yaml-language-server: $schema={}\n",
        local_config_schema_url()
    );
    yaml.push_str("infobases:\n  origin:\n");
    yaml.push_str(&format!("    connection: '{}'\n", escape_yaml(&connection)));
    if let Some(user) = &request.user {
        yaml.push_str(&format!("    user: '{}'\n", escape_yaml(user)));
    }
    if let Some(password) = &request.password {
        yaml.push_str(&format!("    password: '{}'\n", escape_yaml(password)));
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
            "clone connection must not contain embedded credentials; use --user and --password"
                .to_owned(),
        ));
    }

    let args = V8Connection::from_connection_string(connection).args();
    if args.iter().any(|arg| is_embedded_auth_arg(arg)) {
        return Err(AppError::Validation(
            "clone connection must not contain embedded credentials; use --user and --password"
                .to_owned(),
        ));
    }
    Ok(())
}

fn is_embedded_auth_arg(arg: &str) -> bool {
    let key = arg.split_once('=').map_or(arg, |(key, _)| key);
    matches!(key.to_ascii_lowercase().as_str(), "/n" | "-n" | "/p" | "-p")
}

/// Исход попытки выгрузки в трёх признаках. Названы полями, а не позициями: три подряд
/// идущих `bool` переставляются молча, а перестановка здесь меняет ответ.
struct BootstrapOutcome {
    ok: bool,
    dumped: bool,
    provider_dispatched: bool,
    message: Option<String>,
}

/// Исход, выведенный из ответа выгрузки.
///
/// `dumped` требует обоих признаков: ответ превью тоже успешен, но выгрузки в нём не было.
/// Выдумывать это различие не приходится — его называет сама выгрузка.
fn outcome_of(dump: &DumpResult, message: Option<String>) -> BootstrapOutcome {
    BootstrapOutcome {
        ok: dump.ok,
        dumped: dump.ok && dump.provider_dispatched,
        provider_dispatched: dump.provider_dispatched,
        message,
    }
}

fn bootstrap_result(
    started: Instant,
    paths: &BootstrapPaths,
    outcome: BootstrapOutcome,
    warnings: Vec<String>,
) -> BootstrapResult {
    BootstrapResult {
        ok: outcome.ok,
        path: paths.config_path.clone(),
        local_path: paths.local_config_path.clone(),
        gitignore_path: paths.gitignore_path.clone(),
        source_dir: paths.source_dir.clone(),
        dump_target_path: paths.source_dir.clone(),
        dumped: outcome.dumped,
        provider_dispatched: outcome.provider_dispatched,
        warnings,
        message: outcome.message,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::context::CommandName;
    use crate::use_cases::result::UseCaseErrorKind;

    fn request(project_dir: &Path, connection: &str) -> BootstrapRequest {
        BootstrapRequest {
            project_dir: project_dir.to_path_buf(),
            connection: connection.to_owned(),
            platform_version: "8.3.27".to_owned(),
            platform_path: Some(PathBuf::from("/opt/1cv8")),
            user: Some("Admin".to_owned()),
            password: Some("secret".to_owned()),
            source_dir: PathBuf::from("src/configuration"),
            force: false,
            dry_run: true,
        }
    }

    /// Два пути сборки настроек дают одно и то же.
    ///
    /// Превью разбирает текст в памяти, боевой прогон читает его же с диска. Разойтись они
    /// могут только в загрузчике — в разрезе между чтением файлов и разбором, — и увидеть
    /// это можно, лишь сличив оба пути. Сличается весь разобранный документ, а не
    /// выбранные поля; за его границами остаются `provider_origins` (он `serde(skip)`) и
    /// предупреждения загрузчика.
    #[test]
    fn planned_settings_equal_the_settings_read_back_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project_dir = std::fs::canonicalize(dir.path()).expect("canonical");
        let request = request(&project_dir, "File=/tmp/source ib");
        let paths = BootstrapPaths::new(&project_dir, &request.source_dir);

        let main = render_main_config(&paths, &request);
        let local = render_local_config(&request);
        let text = ProjectText {
            project: &main,
            local_overlay: &local,
        };

        let planned = planned_settings(
            &paths,
            ProjectText {
                project: text.project,
                local_overlay: text.local_overlay,
            },
        )
        .expect("planned settings");
        write_bootstrap_files(&paths, &text).expect("written");
        let applied = written_settings(&paths).expect("settings on disk");

        assert_eq!(
            serde_yaml::to_value(&planned.config).expect("planned as value"),
            serde_yaml::to_value(&applied.config).expect("applied as value"),
        );
    }

    /// Боевой план клона во временном каталоге: запись и выгрузка ещё впереди.
    fn written_plan(dir: &Path) -> ClonePlan {
        let project_dir = std::fs::canonicalize(dir).expect("canonical");
        let mut request = request(&project_dir, "File=/tmp/source ib");
        request.dry_run = false;
        plan(request).unwrap_or_else(|failure| panic!("plan: {}", failure.error.message()))
    }

    /// Под замком цели проверяются заново: клон, написавший проект между чужим планом и
    /// замком, не перезаписывается.
    #[test]
    fn the_targets_are_checked_again_under_the_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan = written_plan(dir.path());
        std::fs::write(&plan.paths.config_path, "written by another clone").expect("rival");

        let failure = execute(&ExecutionContext::cli(CommandName::Bootstrap), &plan)
            .expect_err("the target appeared after the plan");

        assert!(failure
            .error
            .message()
            .contains("clone target already exists"));
        assert_eq!(
            std::fs::read_to_string(&plan.paths.config_path).expect("rival"),
            "written by another clone"
        );
    }

    /// Отмена, пришедшая до записи, не оставляет полупроекта.
    #[test]
    fn a_cancelled_clone_writes_no_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan = written_plan(dir.path());
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Bootstrap).with_cancellation(cancellation);

        let failure = execute(&context, &plan).expect_err("cancelled before the write");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Cancelled);
        assert!(!plan.paths.config_path.exists());
        assert!(!plan.paths.local_config_path.exists());
    }

    /// Замок взят по настройкам плана, и написанный проект обязан назвать тот же
    /// `workPath`: иначе выгрузка шла бы под замком чужого каталога.
    #[test]
    fn a_written_project_that_names_another_work_path_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut plan = written_plan(dir.path());
        plan.settings.config.work_path = dir.path().join("elsewhere");

        let failure = execute(&ExecutionContext::cli(CommandName::Bootstrap), &plan)
            .expect_err("the lock and the project disagree");

        assert!(failure.error.message().contains("but the plan locked"));
        let payload = failure.payload.expect("payload");
        assert!(!payload.dumped);
        assert!(!payload.provider_dispatched);
    }

    /// Разрешение пути ничего не создаёт: каталог заводит замок, и до него команда
    /// ещё вправе отказать, не тронув диска.
    #[test]
    fn resolving_a_project_directory_creates_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project_dir = dir.path().join("nested").join("project");

        let resolved = resolve_project_dir(&project_dir).expect("resolved");

        assert!(!project_dir.exists());
        assert_eq!(
            resolved,
            std::fs::canonicalize(dir.path())
                .expect("canonical root")
                .join("nested")
                .join("project")
        );
    }
}
