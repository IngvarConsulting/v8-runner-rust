use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crate::config::schema::main_config_schema_url;
use crate::domain::config_init::{ConfigInitResult, ConfigInitSourceSet};
use crate::support::edt_project::{self, EdtProjectKind};
use crate::support::error::AppError;
use crate::support::path::{is_safe_path_segment, nearest_existing_canonical_path};
use crate::support::source_descriptor::{
    self, SourceDescriptorParseError, SourceDescriptorPurpose, SourceSetRootScanError,
};

const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";
const LOCAL_CONFIG_SCHEMA_MODEL_LINE: &str = "# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInitRequest {
    pub project_dir: PathBuf,
    pub output_path: PathBuf,
    pub force: bool,
    pub connection: Option<String>,
    pub format: ConfigFormatRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormatRequest {
    Auto,
    Designer,
    Edt,
}

pub fn execute(request: &ConfigInitRequest) -> Result<ConfigInitResult, AppError> {
    let started = Instant::now();
    let project_dir = std::fs::canonicalize(&request.project_dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to resolve project directory '{}': {error}",
            request.project_dir.display()
        ))
    })?;
    if !project_dir.is_dir() {
        return Err(AppError::Validation(format!(
            "project directory is not a directory: {}",
            project_dir.display()
        )));
    }

    let output_path = resolve_output_path(&project_dir, &request.output_path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to resolve config output path '{}': {error}",
            request.output_path.display()
        ))
    })?;
    let overwritten = output_path.exists();
    if overwritten && !request.force {
        return Err(AppError::Validation(format!(
            "config file already exists: {} (use --force to overwrite)",
            output_path.display()
        )));
    }

    let output_dir = output_path.parent().unwrap_or(project_dir.as_path());
    std::fs::create_dir_all(output_dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to create config directory '{}': {error}",
            output_dir.display()
        ))
    })?;

    let detected = discover_sources(&project_dir)?;
    let format = choose_format(request.format, &detected);
    let project_source_sets = build_source_sets(&project_dir, &detected, format)?;
    let platform_version = detect_platform_version(&project_dir, format, &project_source_sets)?;
    let warnings = collect_discovery_warnings(&project_dir, format, &project_source_sets)?;
    validate_discovered_source_sets(&project_dir, &project_source_sets)?;
    let source_sets =
        source_sets_relative_to_config_dir(&project_dir, output_dir, &project_source_sets);
    let yaml = render_config(format, &source_sets, platform_version.as_deref());

    let local_path = output_dir.join(LOCAL_CONFIG_FILE_NAME);
    let gitignore_path = output_dir.join(".gitignore");

    std::fs::write(&output_path, yaml).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write config file '{}': {error}",
            output_path.display()
        ))
    })?;
    ensure_local_config(&local_path, request.connection.as_deref())?;
    ensure_gitignore_ignores_local_config(&local_path, &gitignore_path)?;

    Ok(ConfigInitResult {
        ok: true,
        path: output_path.display().to_string(),
        local_path: local_path.display().to_string(),
        gitignore_path: gitignore_path.display().to_string(),
        format: format.as_yaml().to_owned(),
        platform_version,
        source_sets,
        warnings,
        overwritten,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Адрес базы по умолчанию, когда `--connection` не передан: файловая база внутри
/// рабочего каталога.
const DEFAULT_ORIGIN_CONNECTION: &str = "File=build/ib";

/// Местный слой объявляет `origin`: к какой базе подключён каталог, знает эта машина.
/// Существующий слой не переписывается — `origin` дописывается, если не объявлен, а
/// объявленный с другим адресом, чем просили, — отказ, чтобы адрес не потерялся молча.
fn ensure_local_config(path: &Path, connection: Option<&str>) -> Result<(), AppError> {
    let content = if path.exists() {
        let existing = std::fs::read_to_string(path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to read local config file '{}': {error}",
                path.display()
            ))
        })?;
        local_config_with_origin(&existing, connection, path)?
    } else {
        render_local_config_with_origin(connection.unwrap_or(DEFAULT_ORIGIN_CONNECTION))
    };

    std::fs::write(path, content).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write local config file '{}': {error}",
            path.display()
        ))
    })
}

fn with_local_schema_modeline(existing: &str) -> String {
    if existing.trim().is_empty() {
        return render_empty_local_config();
    }

    let mut lines = existing.lines();
    let first_line = lines.next().unwrap_or_default();
    let mut content = String::new();
    if first_line
        .trim_start()
        .starts_with("# yaml-language-server: $schema=")
    {
        content.push_str(LOCAL_CONFIG_SCHEMA_MODEL_LINE);
        content.push('\n');
        let remainder = lines.collect::<Vec<_>>().join("\n");
        if !remainder.is_empty() {
            content.push_str(&remainder);
            content.push('\n');
        }
    } else {
        content.push_str(LOCAL_CONFIG_SCHEMA_MODEL_LINE);
        content.push('\n');
        content.push_str(existing);
        if !existing.ends_with('\n') {
            content.push('\n');
        }
    }
    if yaml_document_is_empty(&content) {
        content.push_str("{}\n");
    }
    content
}

fn render_empty_local_config() -> String {
    format!("{LOCAL_CONFIG_SCHEMA_MODEL_LINE}\n{{}}\n")
}

fn render_origin_block(connection: &str) -> String {
    format!(
        "infobases:\n  origin:\n    connection: '{}'\n",
        escape_yaml(connection)
    )
}

fn render_local_config_with_origin(connection: &str) -> String {
    format!(
        "{LOCAL_CONFIG_SCHEMA_MODEL_LINE}\n{}",
        render_origin_block(connection)
    )
}

fn local_config_with_origin(
    existing: &str,
    connection: Option<&str>,
    path: &Path,
) -> Result<String, AppError> {
    let content = with_local_schema_modeline(existing);
    let document: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|error| {
        AppError::Validation(format!(
            "local config file '{}' is not valid YAML: {error}",
            path.display()
        ))
    })?;
    let requested = connection;
    let connection = connection.unwrap_or(DEFAULT_ORIGIN_CONNECTION);
    match origin_state(&document) {
        OriginState::Connection(declared) => match requested {
            Some(requested) if requested != declared => Err(AppError::Validation(format!(
                "local config file '{}' already declares infobases.origin.connection = '{declared}'; it is not replaced by --connection '{requested}'",
                path.display()
            ))),
            _ => Ok(content),
        },
        OriginState::Standalone => match requested {
            Some(requested) => Err(AppError::Validation(format!(
                "local config file '{}' already declares infobases.origin as a standalone server; it is not replaced by --connection '{requested}'",
                path.display()
            ))),
            None => Ok(content),
        },
        OriginState::Absent if yaml_document_is_empty(&content) => {
            Ok(render_local_config_with_origin(connection))
        }
        OriginState::Absent if appendable(&content, &document) => {
            let mut content = content;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&render_origin_block(connection));
            Ok(content)
        }
        // Секция без адреса, карта с другими базами или потоковая запись: дописать
        // текстом некуда, документ перезаписывается целиком.
        OriginState::Absent | OriginState::WithoutAddress => {
            rewrite_with_origin(document, connection, path)
        }
    }
}

/// Что местный слой уже говорит об `origin`.
enum OriginState {
    /// Ни картой, ни прежним ключом секция не объявлена.
    Absent,
    /// Секция есть, а адреса в ней нет — так писал прежний `bootstrap`: только
    /// учётные данные в местном слое.
    WithoutAddress,
    /// Объявлен адрес.
    Connection(String),
    /// Объявлен автономный сервер — адрес ему не нужен.
    Standalone,
}

fn origin_state(document: &serde_yaml::Value) -> OriginState {
    let Some(section) = document
        .get("infobases")
        .and_then(|infobases| infobases.get("origin"))
        .or_else(|| document.get("infobase"))
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return OriginState::Absent;
    };
    let connection = section
        .get("connection")
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !connection.is_empty() {
        OriginState::Connection(connection.to_owned())
    } else if section
        .get("standalone")
        .is_some_and(|standalone| !standalone.is_null())
    {
        OriginState::Standalone
    } else {
        OriginState::WithoutAddress
    }
}

/// Блок `origin` дописывается текстом, когда документ — блочная запись без ключей баз:
/// так сохраняются комментарии и порядок того, что уже написано.
fn appendable(content: &str, document: &serde_yaml::Value) -> bool {
    let body_is_block_mapping = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .is_some_and(|line| !line.starts_with('{'));
    body_is_block_mapping
        && document.get("infobases").is_none()
        && document.get("infobase").is_none()
}

/// Перезаписывает документ с `origin`, получившим адрес: прежний ключ `infobase`
/// переезжает в `infobases.origin`, остальные поля секции сохраняются.
fn rewrite_with_origin(
    mut document: serde_yaml::Value,
    connection: &str,
    path: &Path,
) -> Result<String, AppError> {
    let key = |name: &str| serde_yaml::Value::String(name.to_owned());
    let mapping = document.as_mapping_mut().ok_or_else(|| {
        AppError::Validation(format!(
            "local config file '{}' must be a YAML mapping",
            path.display()
        ))
    })?;
    let existing = mapping
        .get_mut(key("infobases"))
        .and_then(serde_yaml::Value::as_mapping_mut)
        .and_then(|infobases| infobases.remove(key("origin")))
        .or_else(|| mapping.remove(key("infobase")));
    let mut origin = existing
        .and_then(|section| section.as_mapping().cloned())
        .unwrap_or_default();
    origin.insert(
        key("connection"),
        serde_yaml::Value::String(connection.to_owned()),
    );
    let infobases = mapping
        .entry(key("infobases"))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if !infobases.is_mapping() {
        *infobases = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    if let Some(infobases) = infobases.as_mapping_mut() {
        infobases.insert(key("origin"), serde_yaml::Value::Mapping(origin));
    }
    let rendered = serde_yaml::to_string(&document).map_err(|error| {
        AppError::Runtime(format!(
            "failed to render local config file '{}': {error}",
            path.display()
        ))
    })?;
    Ok(format!("{LOCAL_CONFIG_SCHEMA_MODEL_LINE}\n{rendered}"))
}

fn yaml_document_is_empty(content: &str) -> bool {
    matches!(
        serde_yaml::from_str::<serde_yaml::Value>(content),
        Ok(serde_yaml::Value::Null)
    )
}

fn ensure_gitignore_ignores_local_config(
    local_config_path: &Path,
    gitignore_path: &Path,
) -> Result<(), AppError> {
    match crate::platform::git::check_ignored(local_config_path) {
        Some(true) => Ok(()),
        Some(false) => append_local_config_gitignore_pattern(gitignore_path, false),
        None => append_local_config_gitignore_pattern(gitignore_path, true),
    }
}

fn append_local_config_gitignore_pattern(
    path: &Path,
    skip_existing_pattern: bool,
) -> Result<(), AppError> {
    if path.exists() {
        let existing = std::fs::read_to_string(path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to read gitignore file '{}': {error}",
                path.display()
            ))
        })?;
        if skip_existing_pattern && gitignore_mentions_local_config(&existing) {
            return Ok(());
        }

        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("v8project.local.yaml\n");
        std::fs::write(path, content).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write gitignore file '{}': {error}",
                path.display()
            ))
        })?;
        return Ok(());
    }

    std::fs::write(path, "v8project.local.yaml\n").map_err(|error| {
        AppError::Runtime(format!(
            "failed to write gitignore file '{}': {error}",
            path.display()
        ))
    })
}

fn gitignore_mentions_local_config(content: &str) -> bool {
    content.lines().any(|line| {
        let pattern = line.trim();
        !pattern.is_empty()
            && !pattern.starts_with('#')
            && !pattern.starts_with('!')
            && (pattern == LOCAL_CONFIG_FILE_NAME
                || pattern == "/v8project.local.yaml"
                || pattern == "**/v8project.local.yaml")
    })
}

fn resolve_output_path(project_dir: &Path, output_path: &Path) -> std::io::Result<PathBuf> {
    let output_path = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        project_dir.join(output_path)
    };
    nearest_existing_canonical_path(&output_path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectedSources {
    designer: Vec<DetectedSource>,
    edt: Vec<DetectedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetectedSource {
    path: PathBuf,
    purpose: SourcePurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SourcePurpose {
    Configuration,
    Extension,
    ExternalDataProcessors,
    ExternalReports,
}

impl SourcePurpose {
    const fn as_yaml(self) -> &'static str {
        match self {
            Self::Configuration => "CONFIGURATION",
            Self::Extension => "EXTENSION",
            Self::ExternalDataProcessors => "EXTERNAL_DATA_PROCESSORS",
            Self::ExternalReports => "EXTERNAL_REPORTS",
        }
    }

    const fn sort_rank(self) -> u8 {
        match self {
            Self::Configuration => 0,
            Self::Extension => 1,
            Self::ExternalDataProcessors => 2,
            Self::ExternalReports => 3,
        }
    }

    const fn is_external(self) -> bool {
        matches!(self, Self::ExternalDataProcessors | Self::ExternalReports)
    }
}

impl ConfigFormatRequest {
    const fn as_yaml(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Designer => "DESIGNER",
            Self::Edt => "EDT",
        }
    }
}

fn choose_format(
    requested: ConfigFormatRequest,
    detected: &DetectedSources,
) -> ConfigFormatRequest {
    match requested {
        ConfigFormatRequest::Auto => {
            if detected
                .designer
                .iter()
                .any(|source| source.purpose == SourcePurpose::Configuration)
            {
                ConfigFormatRequest::Designer
            } else if detected
                .edt
                .iter()
                .any(|source| source.purpose == SourcePurpose::Configuration)
            {
                ConfigFormatRequest::Edt
            } else if !detected.designer.is_empty() || detected.edt.is_empty() {
                ConfigFormatRequest::Designer
            } else {
                ConfigFormatRequest::Edt
            }
        }
        explicit => explicit,
    }
}

fn discover_sources(project_dir: &Path) -> Result<DetectedSources, AppError> {
    let mut designer = Vec::new();
    let mut edt = Vec::new();
    let mut seen_designer = HashSet::new();
    let mut seen_edt = HashSet::new();
    scan_dir(
        project_dir,
        project_dir,
        &mut designer,
        &mut edt,
        &mut seen_designer,
        &mut seen_edt,
    )?;
    designer.sort_by(|a, b| a.path.cmp(&b.path));
    edt.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(DetectedSources { designer, edt })
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    designer: &mut Vec<DetectedSource>,
    edt: &mut Vec<DetectedSource>,
    seen_designer: &mut HashSet<PathBuf>,
    seen_edt: &mut HashSet<PathBuf>,
) -> Result<(), AppError> {
    if should_skip_dir(root, dir) {
        return Ok(());
    }

    if let Some(purpose) = detect_designer_source_root(root, dir)? {
        let dir = dir.to_path_buf();
        if seen_designer.insert(dir.clone()) {
            designer.push(DetectedSource { path: dir, purpose });
        }
    }

    if let Some(purpose) = detect_designer_external_root(root, dir)? {
        let dir = dir.to_path_buf();
        if seen_designer.insert(dir.clone()) {
            designer.push(DetectedSource { path: dir, purpose });
        }
    }

    if dir.join(".project").is_file() {
        if let Some(purpose) = detect_edt_project_purpose(dir)? {
            if !purpose.is_external() {
                let dir = dir.to_path_buf();
                if seen_edt.insert(dir.clone()) {
                    edt.push(DetectedSource { path: dir, purpose });
                }
            }
            return Ok(());
        }
    }

    if let Some(purpose) = detect_edt_external_root(dir)? {
        let dir = dir.to_path_buf();
        if seen_edt.insert(dir.clone()) {
            edt.push(DetectedSource { path: dir, purpose });
        }
    }

    let entries = std::fs::read_dir(dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to read source directory '{}': {error}",
            dir.display()
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            AppError::Runtime(format!(
                "failed to read source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            AppError::Runtime(format!(
                "failed to inspect source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if path.is_dir() {
            scan_dir(root, &path, designer, edt, seen_designer, seen_edt)?;
        }
    }

    Ok(())
}

fn should_skip_dir(root: &Path, dir: &Path) -> bool {
    if dir == root {
        return false;
    }
    let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".git" | ".idea" | ".vscode" | ".v8-runner" | "target" | "node_modules" | "dist" | "build"
    )
}

fn has_valid_edt_project_ancestor(root: &Path, start: &Path) -> Result<bool, AppError> {
    let mut current = Some(start);
    while let Some(path) = current {
        if has_valid_edt_project_root(path)? {
            return Ok(true);
        }
        if path == root {
            break;
        }
        current = path.parent();
    }
    Ok(false)
}

fn has_valid_edt_project_root(project_dir: &Path) -> Result<bool, AppError> {
    Ok(
        edt_project::detect_native_ordinary_project_kind(project_dir)
            .map_err(AppError::Validation)?
            .is_some()
            || edt_project::has_native_external_project_layout(project_dir)
                .map_err(AppError::Validation)?,
    )
}

fn detect_designer_source_root(root: &Path, dir: &Path) -> Result<Option<SourcePurpose>, AppError> {
    if has_valid_edt_project_ancestor(root, dir)? {
        return Ok(None);
    }
    let configuration_xml = dir.join("Configuration.xml");
    if !configuration_xml.is_file() {
        return Ok(None);
    }
    detect_designer_purpose(&configuration_xml)
}

fn detect_designer_purpose(configuration_xml: &Path) -> Result<Option<SourcePurpose>, AppError> {
    let detected = classify_source_descriptor_file(configuration_xml)?;
    Ok(match detected {
        Some(SourcePurpose::Configuration | SourcePurpose::Extension) => detected,
        _ => None,
    })
}

fn detect_designer_external_root(
    root: &Path,
    dir: &Path,
) -> Result<Option<SourcePurpose>, AppError> {
    if has_valid_edt_project_ancestor(root, dir)? || dir.join("Configuration.xml").is_file() {
        return Ok(None);
    }

    let mut kinds = HashSet::new();
    let entries =
        source_descriptor::scan_designer_external_root(dir).map_err(map_root_scan_error)?;
    if entries.is_empty() {
        return Ok(None);
    }
    for entry in entries {
        let Some(kind) = entry.purpose else {
            return Ok(None);
        };
        kinds.insert(map_source_descriptor_purpose(kind));
    }

    if kinds.len() != 1 {
        return Ok(None);
    }

    Ok(kinds.into_iter().next())
}

fn detect_edt_project_purpose(project_dir: &Path) -> Result<Option<SourcePurpose>, AppError> {
    let Some(kind) = edt_project::detect_native_ordinary_project_kind(project_dir)
        .map_err(AppError::Validation)?
    else {
        return Ok(None);
    };

    match kind {
        EdtProjectKind::Configuration => Ok(Some(SourcePurpose::Configuration)),
        EdtProjectKind::Extension => Ok(Some(SourcePurpose::Extension)),
        EdtProjectKind::ExternalObjects => Ok(None),
    }
}

fn detect_edt_external_root(dir: &Path) -> Result<Option<SourcePurpose>, AppError> {
    let mut kinds = HashSet::new();
    let entries = source_descriptor::scan_edt_external_root(dir).map_err(map_root_scan_error)?;
    if entries.is_empty() {
        return Ok(None);
    }
    for entry in entries {
        let Some(kind) = entry.purpose else {
            return Ok(None);
        };
        if !kind.is_external() {
            return Ok(None);
        }
        kinds.insert(map_source_descriptor_purpose(kind));
    }

    if kinds.len() != 1 {
        return Ok(None);
    }

    Ok(kinds.into_iter().next())
}
fn classify_source_descriptor_file(path: &Path) -> Result<Option<SourcePurpose>, AppError> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to read source marker '{}': {error}",
            path.display()
        ))
    })?;
    classify_source_descriptor_content(&content, path)
}

fn classify_source_descriptor_content(
    content: &str,
    source_path: &Path,
) -> Result<Option<SourcePurpose>, AppError> {
    let detected =
        source_descriptor::classify_source_descriptor(content).map_err(|error| match error {
            SourceDescriptorParseError::Xml(error) => AppError::Validation(format!(
                "failed to parse source marker '{}': {error}",
                source_path.display()
            )),
            SourceDescriptorParseError::UnexpectedEof => AppError::Validation(format!(
                "failed to parse source marker '{}': unexpected EOF",
                source_path.display()
            )),
        })?;
    Ok(detected.map(map_source_descriptor_purpose))
}

fn map_source_descriptor_purpose(purpose: SourceDescriptorPurpose) -> SourcePurpose {
    match purpose {
        SourceDescriptorPurpose::Configuration => SourcePurpose::Configuration,
        SourceDescriptorPurpose::Extension => SourcePurpose::Extension,
        SourceDescriptorPurpose::ExternalDataProcessors => SourcePurpose::ExternalDataProcessors,
        SourceDescriptorPurpose::ExternalReports => SourcePurpose::ExternalReports,
    }
}

fn map_root_scan_error(error: SourceSetRootScanError) -> AppError {
    match error {
        SourceSetRootScanError::Runtime(message) => AppError::Runtime(message),
        SourceSetRootScanError::Validation(message) => AppError::Validation(message),
    }
}

fn build_source_sets(
    project_dir: &Path,
    detected: &DetectedSources,
    format: ConfigFormatRequest,
) -> Result<Vec<ConfigInitSourceSet>, AppError> {
    let mut selected = match format {
        ConfigFormatRequest::Auto | ConfigFormatRequest::Designer => &detected.designer,
        ConfigFormatRequest::Edt => &detected.edt,
    }
    .clone();

    selected.sort_by(|a, b| {
        a.purpose
            .sort_rank()
            .cmp(&b.purpose.sort_rank())
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut source_sets: Vec<_> = selected
        .iter()
        .enumerate()
        .map(|(index, source)| {
            Ok(ConfigInitSourceSet {
                name: source_set_name(project_dir, source, format, index)?,
                source_type: source.purpose.as_yaml().to_owned(),
                path: relative_path(project_dir, &source.path),
            })
        })
        .collect::<Result<_, AppError>>()?;

    deduplicate_names(&mut source_sets)?;
    Ok(source_sets)
}

fn source_sets_relative_to_config_dir(
    project_dir: &Path,
    config_dir: &Path,
    source_sets: &[ConfigInitSourceSet],
) -> Vec<ConfigInitSourceSet> {
    source_sets
        .iter()
        .map(|source_set| ConfigInitSourceSet {
            name: source_set.name.clone(),
            source_type: source_set.source_type.clone(),
            path: relative_path(config_dir, &project_dir.join(&source_set.path)),
        })
        .collect()
}

fn source_set_name(
    project_dir: &Path,
    source: &DetectedSource,
    format: ConfigFormatRequest,
    index: usize,
) -> Result<String, AppError> {
    if source.purpose == SourcePurpose::Extension {
        return extension_source_set_name(project_dir, &source.path, format);
    }

    Ok(path_source_set_name(&source.path, source.purpose, index))
}

fn path_source_set_name(path: &Path, purpose: SourcePurpose, index: usize) -> String {
    let fallback = match purpose {
        SourcePurpose::Configuration => "main",
        SourcePurpose::Extension => "extension",
        SourcePurpose::ExternalDataProcessors => "external-data-processors",
        SourcePurpose::ExternalReports => "external-reports",
    };
    let raw = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback);
    let normalized = normalize_name(raw);
    if normalized.is_empty() {
        format!("{fallback}-{}", index + 1)
    } else {
        normalized
    }
}

fn normalize_name(raw: &str) -> String {
    let normalized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = normalized.trim_matches('-').to_owned();
    if is_safe_path_segment(&trimmed) {
        trimmed
    } else {
        String::new()
    }
}

fn extension_source_set_name(
    project_dir: &Path,
    source_path: &Path,
    format: ConfigFormatRequest,
) -> Result<String, AppError> {
    let marker_path = match format {
        ConfigFormatRequest::Auto | ConfigFormatRequest::Designer => {
            source_path.join("Configuration.xml")
        }
        ConfigFormatRequest::Edt => edt_project::ordinary_root_marker_path(source_path),
    };
    let raw_name = read_configuration_logical_name(&marker_path)?
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AppError::Validation(format!(
                "extension source-set '{}' must declare configuration logical name in '{}'",
                relative_path(project_dir, source_path),
                marker_path.display()
            ))
        })?;
    if !is_safe_path_segment(&raw_name) {
        return Err(AppError::Validation(format!(
            "extension source-set '{}' declares unsafe configuration logical name '{}'",
            relative_path(project_dir, source_path),
            raw_name
        )));
    }

    Ok(raw_name)
}

fn read_configuration_logical_name(path: &Path) -> Result<Option<String>, AppError> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to read configuration marker '{}': {error}",
            path.display()
        ))
    })?;
    source_descriptor::extract_configuration_logical_name(&content).map_err(|error| match error {
        SourceDescriptorParseError::Xml(error) => AppError::Validation(format!(
            "failed to parse configuration marker '{}': {error}",
            path.display()
        )),
        SourceDescriptorParseError::UnexpectedEof => AppError::Validation(format!(
            "failed to parse configuration marker '{}': unexpected EOF",
            path.display()
        )),
    })
}

fn deduplicate_names(source_sets: &mut [ConfigInitSourceSet]) -> Result<(), AppError> {
    let mut protected_extension_names = HashSet::new();
    for source_set in source_sets
        .iter()
        .filter(|source_set| source_set.source_type == SourcePurpose::Extension.as_yaml())
    {
        if !protected_extension_names.insert(source_set.name.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate extension source-set logical name '{}'",
                source_set.name
            )));
        }
    }

    let mut seen = HashSet::new();
    for source_set in source_sets {
        if source_set.source_type == SourcePurpose::Extension.as_yaml() {
            seen.insert(source_set.name.clone());
            continue;
        }

        let base = if source_set.name.is_empty() {
            "source".to_owned()
        } else {
            source_set.name.clone()
        };
        let mut name = base.clone();
        let mut suffix = 2;
        while protected_extension_names.contains(&name) || !seen.insert(name.clone()) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        source_set.name = name;
    }

    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> String {
    if let Some(relative) = path
        .strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
    {
        return relative.display().to_string();
    }

    let root_components = normalized_components(root);
    let path_components = normalized_components(path);
    let common_len = root_components
        .iter()
        .zip(path_components.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative = PathBuf::new();
    for _ in common_len..root_components.len() {
        relative.push("..");
    }
    for component in &path_components[common_len..] {
        relative.push(component);
    }

    if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        relative.display().to_string()
    }
}

fn normalized_components(path: &Path) -> Vec<OsString> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => components.push(prefix.as_os_str().to_os_string()),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => match components.last() {
                Some(last) if last != ".." => {
                    components.pop();
                }
                _ => components.push(OsString::from("..")),
            },
            Component::Normal(part) => components.push(part.to_os_string()),
        }
    }
    components
}

fn render_config(
    format: ConfigFormatRequest,
    source_sets: &[ConfigInitSourceSet],
    platform_version: Option<&str>,
) -> String {
    let mut yaml = String::new();
    yaml.push_str(&format!(
        "# yaml-language-server: $schema={}\n",
        main_config_schema_url()
    ));
    yaml.push_str("# Generated by v8-runner config init\n");
    yaml.push_str("workPath: 'build'\n");
    yaml.push_str(&format!("format: {}\n", format.as_yaml()));
    yaml.push_str("source-set:\n");
    for source_set in source_sets {
        yaml.push_str(&format!("  - name: '{}'\n", escape_yaml(&source_set.name)));
        yaml.push_str(&format!("    type: {}\n", source_set.source_type));
        yaml.push_str(&format!("    path: '{}'\n", escape_yaml(&source_set.path)));
    }
    if let Some(platform_version) = platform_version {
        yaml.push_str("tools:\n");
        yaml.push_str("  platform:\n");
        yaml.push_str(&format!(
            "    version: '{}'\n",
            escape_yaml(platform_version)
        ));
        yaml.push_str("  # client_mcp:\n");
        yaml.push_str("  #   # How long to wait for the client MCP endpoint to come up.\n");
        yaml.push_str("  #   wait_ready_timeout_ms: 300000\n");
    } else {
        yaml.push_str("# tools:\n");
        yaml.push_str("#   client_mcp:\n");
        yaml.push_str("#     # How long to wait for the client MCP endpoint to come up.\n");
        yaml.push_str("#     wait_ready_timeout_ms: 300000\n");
    }
    yaml.push_str("build:\n");
    yaml.push_str("  partialLoadThreshold: 20\n");
    yaml
}

fn validate_discovered_source_sets(
    project_dir: &Path,
    source_sets: &[ConfigInitSourceSet],
) -> Result<(), AppError> {
    if source_sets.is_empty() {
        return Err(AppError::Validation(format!(
            "failed to autodiscover source-set markers in '{}'; add source-set manually",
            project_dir.display()
        )));
    }
    if !source_sets
        .iter()
        .any(|source_set| source_set.source_type == "CONFIGURATION")
    {
        return Err(AppError::Validation(
            "autodiscovery did not find a CONFIGURATION source-set; add it manually".to_owned(),
        ));
    }
    Ok(())
}

fn detect_platform_version(
    project_dir: &Path,
    format: ConfigFormatRequest,
    source_sets: &[ConfigInitSourceSet],
) -> Result<Option<String>, AppError> {
    if format != ConfigFormatRequest::Edt {
        return Ok(None);
    }

    let Some(configuration) = source_sets
        .iter()
        .find(|source_set| source_set.source_type == SourcePurpose::Configuration.as_yaml())
    else {
        return Ok(None);
    };
    let source_path = project_dir.join(&configuration.path);
    let Some(manifest) =
        edt_project::read_project_manifest_from_dir(&source_path).map_err(AppError::Validation)?
    else {
        return Ok(None);
    };

    Ok(manifest.runtime_version)
}

fn collect_discovery_warnings(
    project_dir: &Path,
    format: ConfigFormatRequest,
    source_sets: &[ConfigInitSourceSet],
) -> Result<Vec<String>, AppError> {
    if format != ConfigFormatRequest::Edt {
        return Ok(Vec::new());
    }

    let mut warnings = Vec::new();
    for source_set in source_sets {
        if source_set.source_type != SourcePurpose::Extension.as_yaml() {
            continue;
        }

        let source_path = project_dir.join(&source_set.path);
        let Some(manifest) = edt_project::read_project_manifest_from_dir(&source_path)
            .map_err(AppError::Validation)?
        else {
            continue;
        };
        if manifest.base_project.is_none() {
            warnings.push(format!(
                "EDT extension source-set '{}' does not declare Base-Project in 'DT-INF/PROJECT.PMF': {}",
                source_set.name,
                source_set.path
            ));
        }
    }

    Ok(warnings)
}

fn escape_yaml(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::{discover_sources, execute, ConfigFormatRequest, ConfigInitRequest, SourcePurpose};
    use crate::config::loader::load_config;
    use crate::config::model::InfobaseSelector;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn canonical(path: &Path) -> std::path::PathBuf {
        std::fs::canonicalize(path).expect("canonical test path")
    }

    fn create_native_edt_external_project(
        project_dir: &Path,
        name: &str,
        descriptor_xml: &str,
        base_project: Option<&str>,
    ) {
        write_file(
            &project_dir.join(".project"),
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>{}</nature>\n  </natures>\n</projectDescription>\n",
                crate::support::edt_project::V8_EXTERNAL_OBJECTS_NATURE
            ),
        );
        let base_line = base_project
            .map(|value| format!("Base-Project: {value}\n"))
            .unwrap_or_default();
        write_file(
            &project_dir.join("DT-INF").join("PROJECT.PMF"),
            &format!("{base_line}Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n"),
        );
        write_file(&project_dir.join("src").join("root.xml"), descriptor_xml);
    }

    fn init_git_repo(path: &Path) {
        let status = Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(path)
            .status()
            .expect("run git init");
        assert!(status.success());
    }

    fn create_native_edt_project(
        project_dir: &Path,
        name: &str,
        nature: &str,
        base_project: Option<&str>,
    ) {
        create_native_edt_project_with_config_name(project_dir, name, name, nature, base_project);
    }

    fn create_native_edt_project_with_config_name(
        project_dir: &Path,
        project_name: &str,
        config_name: &str,
        nature: &str,
        base_project: Option<&str>,
    ) {
        write_file(
            &project_dir.join(".project"),
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{nature}</nature>\n  </natures>\n</projectDescription>\n"
            ),
        );
        let base_line = base_project
            .map(|value| format!("Base-Project: {value}\n"))
            .unwrap_or_default();
        write_file(
            &project_dir.join("DT-INF").join("PROJECT.PMF"),
            &format!("{base_line}Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n"),
        );
        write_file(
            &project_dir
                .join("src")
                .join("Configuration")
                .join("Configuration.mdo"),
            &format!(
                "<mdclass:Configuration xmlns:mdclass=\"http://g5.1c.ru/v8/dt/metadata/mdclass\"><name>{config_name}</name></mdclass:Configuration>\n"
            ),
        );
        write_file(
            &project_dir
                .join("src")
                .join("Configuration")
                .join("Module.bsl"),
            "Procedure Test()\nEndProcedure\n",
        );
    }

    #[test]
    fn creates_config_from_designer_sources() {
        let dir = tempdir().expect("tempdir");
        let main = dir.path().join("src").join("main");
        let ext = dir.path().join("extensions").join("sales");
        std::fs::create_dir_all(&main).expect("main");
        std::fs::create_dir_all(&ext).expect("ext");
        std::fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            ext.join("Configuration.xml"),
            "<Configuration><Properties><Name>SalesAddon</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
        )
        .expect("ext xml");

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Auto,
        })
        .expect("init config");

        assert_eq!(result.format, "DESIGNER");
        assert_eq!(result.source_sets.len(), 2);
        assert!(result.source_sets.iter().any(|source| {
            source.name == "SalesAddon"
                && source.path == "extensions/sales"
                && source.source_type == "EXTENSION"
        }));
        assert!(std::fs::read_to_string(dir.path().join("v8project.yaml"))
            .expect("config")
            .contains("name: 'SalesAddon'"));
        assert!(std::fs::read_to_string(dir.path().join("v8project.yaml"))
            .expect("config")
            .contains("type: EXTENSION"));
    }

    #[test]
    fn quotes_generated_source_set_name_so_config_can_be_reloaded() {
        let dir = tempdir().expect("tempdir");
        let main = dir.path().join("src").join("main");
        let ext = dir.path().join("extensions").join("sales");
        std::fs::create_dir_all(&main).expect("main");
        std::fs::create_dir_all(&ext).expect("ext");
        std::fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            ext.join("Configuration.xml"),
            "<Configuration><Properties><Name>#SalesAddon</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
        )
        .expect("ext xml");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let config_text =
            std::fs::read_to_string(dir.path().join("v8project.yaml")).expect("generated config");
        assert!(config_text.contains("name: '#SalesAddon'"));
        let loaded = load_config(
            Some(
                dir.path()
                    .join("v8project.yaml")
                    .to_str()
                    .expect("config path"),
            ),
            None,
            &InfobaseSelector::Default,
        )
        .map(|loaded| loaded.config)
        .expect("generated config should reload");
        assert!(loaded
            .source_sets
            .iter()
            .any(|source_set| source_set.name == "#SalesAddon"));
    }

    #[test]
    fn rejects_extension_logical_name_with_control_characters() {
        let dir = tempdir().expect("tempdir");
        let main = dir.path().join("src").join("main");
        let ext = dir.path().join("extensions").join("sales");
        std::fs::create_dir_all(&main).expect("main");
        std::fs::create_dir_all(&ext).expect("ext");
        std::fs::write(main.join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            ext.join("Configuration.xml"),
            "<Configuration><Properties><Name>Sales\nAddon</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
        )
        .expect("ext xml");

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("control characters are unsafe");

        assert!(error
            .to_string()
            .contains("declares unsafe configuration logical name"));
        assert!(!dir.path().join("v8project.yaml").exists());
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("v8project.yaml"), "existing").expect("existing");

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("should fail");

        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn primary_config_write_failure_does_not_create_local_side_effects() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::create_dir(dir.path().join("v8project.yaml")).expect("config path directory");

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: true,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("directory output path cannot be written as file");

        assert!(error.to_string().contains("failed to write config file"));
        assert!(!dir.path().join("v8project.local.yaml").exists());
        assert!(!dir.path().join(".gitignore").exists());
    }

    #[test]
    fn creates_empty_local_overlay_and_gitignore_entry() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        assert_eq!(
            result.local_path,
            canonical(dir.path())
                .join("v8project.local.yaml")
                .display()
                .to_string()
        );
        assert_eq!(
            result.gitignore_path,
            canonical(dir.path())
                .join(".gitignore")
                .display()
                .to_string()
        );
        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        assert_eq!(
            local_config,
            format!(
                "{}\ninfobases:\n  origin:\n    connection: 'File=build/ib'\n",
                super::LOCAL_CONFIG_SCHEMA_MODEL_LINE
            ),
            "the local layer declares origin; the project file names no base"
        );
        let project_config =
            std::fs::read_to_string(dir.path().join("v8project.yaml")).expect("project config");
        assert!(!project_config.contains("infobase"), "{project_config}");
        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(gitignore, "v8project.local.yaml\n");
    }

    #[test]
    fn preserves_existing_local_overlay_and_gitignore_pattern() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            dir.path().join("v8project.local.yaml"),
            "# yaml-language-server: $schema=https://old.example/schema.json\nworkPath: local-work\n",
        )
        .expect("local config");
        std::fs::write(
            dir.path().join(".gitignore"),
            "# local state\n**/v8project.local.yaml\n",
        )
        .expect("gitignore");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        assert!(local_config.starts_with(super::LOCAL_CONFIG_SCHEMA_MODEL_LINE));
        assert!(local_config.contains("workPath: local-work"));
        assert!(!local_config.contains("old.example"));
        assert!(
            local_config.ends_with("infobases:\n  origin:\n    connection: 'File=build/ib'\n"),
            "origin is appended to a layer that does not declare it:\n{local_config}"
        );
        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(gitignore, "# local state\n**/v8project.local.yaml\n");
    }

    #[test]
    fn keeps_an_existing_origin_and_refuses_another_connection_for_it() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        let existing = format!(
            "{}\ninfobases:\n  origin:\n    connection: 'File=/srv/ib'\n    user: Admin\n",
            super::LOCAL_CONFIG_SCHEMA_MODEL_LINE
        );
        std::fs::write(dir.path().join("v8project.local.yaml"), &existing).expect("local config");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");
        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        assert_eq!(local_config, existing, "a declared origin is left as it is");

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: true,
            connection: Some("File=/other/ib".to_owned()),
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("another address for a declared origin");
        let message = error.to_string();
        assert!(
            message.contains("already declares infobases.origin.connection = 'File=/srv/ib'"),
            "{message}"
        );
        assert!(message.contains("File=/other/ib"), "{message}");
    }

    /// Прежний `bootstrap` писал в местный слой только учётные данные под `infobase:`;
    /// `config init --force` даёт такой секции адрес и переводит её в `infobases.origin`.
    #[test]
    fn gives_an_address_to_an_origin_that_has_only_credentials() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            dir.path().join("v8project.local.yaml"),
            "infobase:\n  user: Admin\n  password: secret\n",
        )
        .expect("local config");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: Some("Srvr=srv;Ref=erp".to_owned()),
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        let document: serde_yaml::Value = serde_yaml::from_str(&local_config).expect("yaml");
        assert_eq!(
            document["infobases"]["origin"]["connection"],
            "Srvr=srv;Ref=erp"
        );
        assert_eq!(document["infobases"]["origin"]["user"], "Admin");
        assert_eq!(document["infobases"]["origin"]["password"], "secret");
        assert!(
            document.get("infobase").is_none(),
            "the synonym key is gone:\n{local_config}"
        );
        assert!(local_config.starts_with(super::LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    }

    #[test]
    fn a_null_infobases_key_is_replaced_by_the_map() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(dir.path().join("v8project.local.yaml"), "infobases:\n")
            .expect("local config");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        let document: serde_yaml::Value =
            serde_yaml::from_str(&local_config).expect("still one document");
        assert_eq!(
            document["infobases"]["origin"]["connection"],
            "File=build/ib"
        );
    }

    #[test]
    fn adds_origin_to_an_existing_map_of_other_infobases() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            dir.path().join("v8project.local.yaml"),
            "infobases:\n  test:\n    connection: 'File=/srv/test-ib'\n",
        )
        .expect("local config");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: Some("Srvr=srv;Ref=erp".to_owned()),
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let local_config =
            std::fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local config");
        let document: serde_yaml::Value = serde_yaml::from_str(&local_config).expect("yaml");
        assert_eq!(
            document["infobases"]["origin"]["connection"],
            "Srvr=srv;Ref=erp"
        );
        assert_eq!(
            document["infobases"]["test"]["connection"],
            "File=/srv/test-ib"
        );
        assert!(local_config.starts_with(super::LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    }

    #[test]
    fn appends_gitignore_entry_when_existing_pattern_targets_only_nested_local_config() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(dir.path().join(".gitignore"), "docs/v8project.local.yaml\n")
            .expect("gitignore");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(
            gitignore,
            "docs/v8project.local.yaml\nv8project.local.yaml\n"
        );
    }

    #[test]
    fn does_not_append_gitignore_entry_when_git_already_ignores_local_config() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(dir.path().join(".gitignore"), "*.local.yaml\n").expect("gitignore");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(gitignore, "*.local.yaml\n");
    }

    #[test]
    fn appends_gitignore_entry_when_existing_pattern_is_negated() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            dir.path().join(".gitignore"),
            "v8project.local.yaml\n!v8project.local.yaml\n",
        )
        .expect("gitignore");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(
            gitignore,
            "v8project.local.yaml\n!v8project.local.yaml\nv8project.local.yaml\n"
        );
    }

    #[test]
    fn does_not_create_nested_gitignore_when_root_gitignore_covers_output_override_local_config() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("main xml");
        std::fs::write(
            dir.path().join(".gitignore"),
            "config/v8project.local.yaml\n",
        )
        .expect("gitignore");

        execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "config/v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        assert!(dir
            .path()
            .join("config")
            .join("v8project.local.yaml")
            .exists());
        assert!(!dir.path().join("config").join(".gitignore").exists());
        let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).expect("gitignore");
        assert_eq!(gitignore, "config/v8project.local.yaml\n");
    }

    #[test]
    fn extension_detection_uses_designer_xml_marker() {
        let dir = tempdir().expect("tempdir");
        let xml = dir.path().join("Configuration.xml");
        std::fs::write(
            &xml,
            "<Configuration><ObjectBelonging>Adopted</ObjectBelonging></Configuration>",
        )
        .expect("xml");

        assert_eq!(
            super::detect_designer_purpose(&xml).expect("designer purpose"),
            Some(SourcePurpose::Extension)
        );
    }

    #[test]
    fn discovers_nested_designer_sources_without_relying_on_root_structure() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Configuration.xml"), "<Configuration/>").expect("root xml");
        let ext = dir.path().join("packages").join("sales-addon");
        std::fs::create_dir_all(&ext).expect("ext dir");
        std::fs::write(
            ext.join("Configuration.xml"),
            "<Configuration><Properties><Name>SalesAddon</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
        )
        .expect("ext xml");

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        assert_eq!(result.source_sets.len(), 2);
        assert!(result
            .source_sets
            .iter()
            .any(|source| source.path == "." && source.source_type == "CONFIGURATION"));
        assert!(result.source_sets.iter().any(|source| {
            source.name == "SalesAddon"
                && source.path == "packages/sales-addon"
                && source.source_type == "EXTENSION"
        }));
    }

    #[test]
    fn detects_native_edt_extension_name_from_configuration_file() {
        let dir = tempdir().expect("tempdir");
        let config_project = dir.path().join("workspace").join("cfg-project");
        let extension_project = dir.path().join("workspace").join("addon-project");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_project_with_config_name(
            &extension_project,
            "sales-project",
            "sales_addon",
            crate::support::edt_project::V8_EXTENSION_NATURE,
            Some("configuration"),
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Auto,
        })
        .expect("init config");

        assert_eq!(result.format, "EDT");
        assert_eq!(result.platform_version.as_deref(), Some("8.3.27"));
        assert!(result.source_sets.iter().any(|source| {
            source.path == "workspace/cfg-project" && source.source_type == "CONFIGURATION"
        }));
        assert!(result.source_sets.iter().any(|source| {
            source.name == "sales_addon"
                && source.path == "workspace/addon-project"
                && source.source_type == "EXTENSION"
        }));
    }

    #[test]
    fn native_edt_extension_without_base_project_is_detected_with_warning() {
        let dir = tempdir().expect("tempdir");
        let config_project = dir.path().join("workspace").join("cfg-project");
        let extension_project = dir.path().join("workspace").join("addon-project");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_project(
            &extension_project,
            "sales",
            crate::support::edt_project::V8_EXTENSION_NATURE,
            None,
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Auto,
        })
        .expect("init config");

        assert_eq!(result.format, "EDT");
        assert_eq!(result.platform_version.as_deref(), Some("8.3.27"));
        assert!(result.source_sets.iter().any(|source| {
            source.path == "workspace/cfg-project" && source.source_type == "CONFIGURATION"
        }));
        assert!(result.source_sets.iter().any(|source| {
            source.name == "sales"
                && source.path == "workspace/addon-project"
                && source.source_type == "EXTENSION"
        }));
        assert!(result
            .warnings
            .iter()
            .any(|warning| { warning.contains("sales") && warning.contains("Base-Project") }));
    }

    #[test]
    fn discovers_designer_external_aggregate_root_from_top_level_descriptors() {
        let dir = tempdir().expect("tempdir");
        write_file(&dir.path().join("Configuration.xml"), "<Configuration/>");
        write_file(
            &dir.path().join("tools").join("alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        );
        write_file(
            &dir.path().join("tools").join("beta.xml"),
            "<MetaDataObject><ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor></MetaDataObject>",
        );
        write_file(
            &dir.path().join("tools").join("nested").join("ignored.xml"),
            "<ExternalReport><Properties><Name>Ignored</Name></Properties></ExternalReport>",
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        assert!(result.source_sets.iter().any(|source| {
            source.path == "tools" && source.source_type == "EXTERNAL_DATA_PROCESSORS"
        }));
    }

    #[test]
    fn ignores_mixed_designer_external_root() {
        let dir = tempdir().expect("tempdir");
        write_file(&dir.path().join("Configuration.xml"), "<Configuration/>");
        write_file(
            &dir.path().join("mixed").join("alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        );
        write_file(
            &dir.path().join("mixed").join("beta.xml"),
            "<ExternalReport><Properties><Name>Beta</Name></Properties></ExternalReport>",
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        assert_eq!(result.source_sets.len(), 1);
        assert_eq!(result.source_sets[0].source_type, "CONFIGURATION");
    }

    #[test]
    fn detects_edt_external_aggregate_root_from_direct_child_projects() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let config_project = workspace.join("cfg");
        let external_root = workspace.join("processors");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_external_project(
            &external_root.join("alpha"),
            "alpha",
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
            Some("configuration"),
        );
        create_native_edt_external_project(
            &external_root.join("beta"),
            "beta",
            "<MetaDataObject><ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor></MetaDataObject>",
            Some("configuration"),
        );
        create_native_edt_external_project(
            &external_root.join("nested").join("gamma"),
            "gamma",
            "<ExternalReport><Properties><Name>Gamma</Name></Properties></ExternalReport>",
            Some("configuration"),
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Auto,
        })
        .expect("init config");

        assert_eq!(result.format, "EDT");
        assert!(result.source_sets.iter().any(|source| {
            source.path == "workspace/cfg" && source.source_type == "CONFIGURATION"
        }));
        assert!(result.source_sets.iter().any(|source| {
            source.path == "workspace/processors"
                && source.source_type == "EXTERNAL_DATA_PROCESSORS"
        }));
    }

    #[test]
    fn edt_internal_markers_do_not_create_designer_candidates() {
        let dir = tempdir().expect("tempdir");
        let project = dir.path().join("workspace").join("cfg");
        create_native_edt_project(
            &project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );

        let detected = discover_sources(dir.path()).expect("discover sources");

        assert!(detected.designer.is_empty());
        assert_eq!(detected.edt.len(), 1);
        assert_eq!(detected.edt[0].purpose, SourcePurpose::Configuration);
    }

    #[test]
    fn external_only_autodiscovery_fails_without_phantom_configuration() {
        let dir = tempdir().expect("tempdir");
        write_file(
            &dir.path().join("tools").join("alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        );

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("external-only autodetect must fail");

        assert!(error
            .to_string()
            .contains("did not find a CONFIGURATION source-set"));
    }

    #[test]
    fn ambiguous_edt_external_root_is_not_detected() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let config_project = workspace.join("cfg");
        let external_root = workspace.join("processors");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_external_project(
            &external_root.join("alpha"),
            "alpha",
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
            Some("configuration"),
        );
        create_native_edt_external_project(
            &external_root.join("beta"),
            "beta",
            "<Form/>",
            Some("configuration"),
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Edt,
        })
        .expect("init config");

        assert_eq!(result.source_sets.len(), 1);
        assert_eq!(result.source_sets[0].source_type, "CONFIGURATION");
    }

    #[test]
    fn edt_external_root_requires_canonical_root_descriptor() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let config_project = workspace.join("cfg");
        let external_root = workspace.join("processors");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_external_project(
            &external_root.join("alpha"),
            "alpha",
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
            Some("configuration"),
        );
        std::fs::create_dir_all(external_root.join("alpha").join("src").join("nested"))
            .expect("nested dir");
        std::fs::rename(
            external_root.join("alpha").join("src").join("root.xml"),
            external_root
                .join("alpha")
                .join("src")
                .join("nested")
                .join("alpha.xml"),
        )
        .expect("move root descriptor");

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Edt,
        })
        .expect("init config");

        assert_eq!(result.source_sets.len(), 1);
        assert_eq!(result.source_sets[0].path, "workspace/cfg");
        assert_eq!(result.source_sets[0].source_type, "CONFIGURATION");
    }

    #[test]
    fn edt_external_root_requires_base_project_in_manifest() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let config_project = workspace.join("cfg");
        let external_root = workspace.join("processors");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        create_native_edt_external_project(
            &external_root.join("alpha"),
            "alpha",
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
            None,
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Edt,
        })
        .expect("init config");

        assert_eq!(result.source_sets.len(), 1);
        assert_eq!(result.source_sets[0].path, "workspace/cfg");
        assert_eq!(result.source_sets[0].source_type, "CONFIGURATION");
    }

    #[test]
    fn auto_prefers_edt_when_designer_only_has_external_roots() {
        let dir = tempdir().expect("tempdir");
        let config_project = dir.path().join("workspace").join("cfg");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        write_file(
            &dir.path().join("tools").join("alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Auto,
        })
        .expect("init config");

        assert_eq!(result.format, "EDT");
        assert_eq!(result.source_sets.len(), 1);
        assert_eq!(result.source_sets[0].path, "workspace/cfg");
        assert_eq!(result.source_sets[0].source_type, "CONFIGURATION");
    }

    #[test]
    fn invalid_root_project_marker_does_not_hide_nested_edt_project() {
        let dir = tempdir().expect("tempdir");
        write_file(&dir.path().join(".project"), "<root/>");
        let config_project = dir.path().join("workspace").join("cfg");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Edt,
        })
        .expect("init config");

        assert!(result.source_sets.iter().any(|source| {
            source.path == "workspace/cfg" && source.source_type == "CONFIGURATION"
        }));
    }

    #[test]
    fn parseable_non_native_project_marker_does_not_hide_designer_sources() {
        let dir = tempdir().expect("tempdir");
        write_file(
            &dir.path().join(".project"),
            "<projectDescription><name>legacy</name></projectDescription>",
        );
        write_file(
            &dir.path().join("designer").join("Configuration.xml"),
            "<Configuration/>",
        );

        let detected = discover_sources(dir.path()).expect("discover sources");

        assert_eq!(detected.designer.len(), 1);
        assert_eq!(detected.designer[0].path, dir.path().join("designer"));
        assert_eq!(detected.designer[0].purpose, SourcePurpose::Configuration);
    }

    #[test]
    fn malformed_configuration_marker_reports_specific_path() {
        let dir = tempdir().expect("tempdir");
        let marker = dir.path().join("Configuration.xml");
        write_file(&marker, "<Configuration>");

        let error = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect_err("malformed marker must fail");

        let error = error.to_string();
        assert!(error.contains("failed to parse source marker"));
        assert!(error.contains("Configuration.xml"));
    }

    #[test]
    fn edt_aggregate_root_does_not_hide_nested_configuration_project() {
        let dir = tempdir().expect("tempdir");
        let external_root = dir.path().join("processors");
        create_native_edt_external_project(
            &external_root.join("alpha"),
            "alpha",
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
            Some("configuration"),
        );
        create_native_edt_external_project(
            &external_root.join("beta"),
            "beta",
            "<ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor>",
            Some("configuration"),
        );
        let config_project = external_root.join("apps").join("cfg");
        create_native_edt_project(
            &config_project,
            "configuration",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Edt,
        })
        .expect("init config");

        assert!(result.source_sets.iter().any(|source| {
            source.path == "processors" && source.source_type == "EXTERNAL_DATA_PROCESSORS"
        }));
        assert!(result.source_sets.iter().any(|source| {
            source.path == "processors/apps/cfg" && source.source_type == "CONFIGURATION"
        }));
    }

    #[test]
    fn generated_config_round_trips_through_loader() {
        let dir = tempdir().expect("tempdir");
        let main = dir.path().join("Configuration.xml");
        std::fs::write(&main, "<Configuration/>").expect("main xml");

        let result = execute(&ConfigInitRequest {
            project_dir: dir.path().to_path_buf(),
            output_path: "v8project.yaml".into(),
            force: false,
            connection: None,
            format: ConfigFormatRequest::Designer,
        })
        .expect("init config");

        let config = load_config(Some(&result.path), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(
            config.infobase.connection,
            format!("File={}", canonical(dir.path()).join("build/ib").display())
        );
    }
}
