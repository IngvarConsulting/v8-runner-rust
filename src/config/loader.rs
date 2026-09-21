use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::config::model::{
    is_infobase_name, AppConfig, InfobaseConfig, InfobaseSelector, DEFAULT_INFOBASE_NAME,
    INFOBASE_NAME_PATTERN,
};
use crate::config::schema::{
    validate_local_overlay_schema_boundary, validate_main_config_schema_boundary,
};
use crate::config::validate::{
    validate, validate_infobase_export, validate_prepared_test, validate_read_only,
    validate_tools_download_bootstrap, ConfigValidationError,
};
use crate::support::path::normalize_windows_verbatim_path;

pub const DEFAULT_CONFIG_FILE_NAME: &str = "v8project.yaml";
pub const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("config file not found: {0}")]
    NotFound(String),

    #[error("failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("failed to parse YAML config: {0}")]
    ParseError(#[from] serde_yaml::Error),

    #[error("config validation failed: {0}")]
    ValidationError(#[from] ConfigValidationError),

    #[error("{0} is a local overlay and cannot be used as --config")]
    LocalOverlayAsPrimaryConfig(String),

    #[error("local config overlay cannot override project identity key '{0}'")]
    LocalOverlayForbiddenKey(&'static str),

    #[error("local config overlay does not support top-level key '{0}'")]
    LocalOverlayUnsupportedKey(String),

    #[error("local config overlay contains unsupported key or value: {0}")]
    LocalOverlayUnsupportedShape(String),

    #[error("config contains unsupported key or value: {0}")]
    UnsupportedShape(String),
}

/// A loaded configuration and what the loader wants the user to hear about it.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: AppConfig,
    /// Keys the loader still accepts for one release cycle under their old name. The
    /// caller shows them: in text as a warning of its own node, in JSON in the
    /// envelope's `warnings`.
    pub warnings: Vec<String>,
}

pub fn load_config(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
) -> Result<LoadedConfig, ConfigLoadError> {
    load_config_with_mode(
        config_path,
        workdir_override,
        selector,
        ConfigValidationMode::Full,
    )
}

/// Load and fully validate a project without creating workPath for a preview.
pub fn load_config_for_preview(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
) -> Result<LoadedConfig, ConfigLoadError> {
    load_config_with_mode(
        config_path,
        workdir_override,
        selector,
        ConfigValidationMode::Preview,
    )
}

pub fn load_config_for_tools_download(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
) -> Result<LoadedConfig, ConfigLoadError> {
    load_config_with_mode(
        config_path,
        workdir_override,
        selector,
        ConfigValidationMode::ToolsDownload,
    )
}

pub fn load_config_for_prepared_test(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
) -> Result<LoadedConfig, ConfigLoadError> {
    load_config_with_mode(
        config_path,
        workdir_override,
        selector,
        ConfigValidationMode::PreparedTest,
    )
}

pub fn load_config_for_infobase_export(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
) -> Result<LoadedConfig, ConfigLoadError> {
    load_config_with_mode(
        config_path,
        workdir_override,
        selector,
        ConfigValidationMode::InfobaseExport,
    )
}

enum ConfigValidationMode {
    Full,
    Preview,
    InfobaseExport,
    PreparedTest,
    ToolsDownload,
}

fn load_config_with_mode(
    config_path: Option<&str>,
    workdir_override: Option<&str>,
    selector: &InfobaseSelector,
    validation_mode: ConfigValidationMode,
) -> Result<LoadedConfig, ConfigLoadError> {
    let path = resolve_config_path(config_path)?;
    reject_local_overlay_as_primary_config(&path)?;
    let path = normalize_windows_verbatim_path(&std::fs::canonicalize(&path)?);
    let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut root = read_yaml_file(&path)?;
    reject_legacy_config_keys(&root)?;
    reject_infobases_in_project_file(&root)?;
    let mut warnings = Vec::new();
    warnings.extend(fold_infobase_synonym(
        &mut root,
        ConfigFile::Project(&path),
    )?);

    // Переопределение провайдера попадает в квитанцию вместе с именем файла, который
    // его поставил: отличать проектный выбор от машинно-локального эксперимента нужно
    // именно там, где читают квитанцию.
    let mut provider_origins = provider_override_keys(&root, DEFAULT_CONFIG_FILE_NAME);

    let local_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCAL_CONFIG_FILE_NAME);
    if local_path.exists() {
        let mut overlay = read_yaml_file(&local_path)?;
        reject_legacy_config_keys(&overlay)?;
        reject_local_overlay_keys(&overlay)?;
        validate_local_overlay_schema_boundary(overlay.clone())
            .map_err(|error| ConfigLoadError::LocalOverlayUnsupportedShape(error.to_string()))?;
        warnings.extend(fold_infobase_synonym(&mut overlay, ConfigFile::Local)?);
        provider_origins.extend(provider_override_keys(&overlay, LOCAL_CONFIG_FILE_NAME));
        merge_yaml_values(&mut root, overlay);
    }

    reject_legacy_config_keys(&root)?;
    // Форма слитого документа: `workPath` из местного слоя дополняет проектный файл, а
    // карту `infobases` главная граница читает, не публикуя, — в проектном файле её
    // отвергли выше, до границы.
    validate_main_config_schema_boundary(root.clone())
        .map_err(|error| ConfigLoadError::UnsupportedShape(error.to_string()))?;
    select_infobase(&mut root, selector)?;
    default_base_path_to_config_dir(&mut root, config_dir)?;

    let mut config: AppConfig = serde_yaml::from_value(root)?;
    config.provider_origins = provider_origins
        .into_iter()
        .filter_map(|(key, file)| {
            crate::domain::capability::Operation::parse(&key).map(|operation| (operation, file))
        })
        .collect();
    normalize_config_paths(&mut config, config_dir);

    if let Some(wd) = workdir_override {
        config.work_path = normalize_optional_path(Path::new(wd), config_dir);
    }

    match validation_mode {
        ConfigValidationMode::Full => validate(&config)?,
        ConfigValidationMode::Preview => validate_read_only(&config)?,
        ConfigValidationMode::InfobaseExport => validate_infobase_export(&config)?,
        ConfigValidationMode::PreparedTest => validate_prepared_test(&config)?,
        ConfigValidationMode::ToolsDownload => validate_tools_download_bootstrap(&config)?,
    }
    warnings.extend(direct_gate_declared_but_not_used_yet(&config));
    Ok(LoadedConfig { config, warnings })
}

/// Строка прямого шлюза рядом с секцией `standalone` принимается, но до появления
/// исполнителя по прямому шлюзу (#205) её никто не читает: команды идут через
/// `standalone.gate`. Молчать об этом нельзя — сайт обещает Конфигуратор по этой строке.
fn direct_gate_declared_but_not_used_yet(config: &AppConfig) -> Option<String> {
    if config.infobase.standalone.is_none() || config.infobase.connection.trim().is_empty() {
        return None;
    }
    let name = config
        .infobase_name
        .as_deref()
        .unwrap_or(DEFAULT_INFOBASE_NAME);
    Some(format!(
        "infobases.{name}: the direct gate address in `connection` is declared but not used yet — commands go through standalone.gate until the Designer path arrives (#205)"
    ))
}

fn yaml_key(key: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(key.to_owned())
}

fn root_mapping_mut(
    root: &mut serde_yaml::Value,
) -> Result<&mut serde_yaml::Mapping, ConfigValidationError> {
    root.as_mapping_mut().ok_or_else(|| {
        ConfigValidationError::InvalidYamlRoot(
            "expected a YAML mapping at the document root".to_owned(),
        )
    })
}

/// Карта баз живёт только в местном слое: к какой базе подключён каталог, знает эта
/// машина, а не проект. Отказ называет слой до границы схемы, где ключ был бы просто
/// неизвестным.
fn reject_infobases_in_project_file(root: &serde_yaml::Value) -> Result<(), ConfigValidationError> {
    let Some(mapping) = root.as_mapping() else {
        return Err(ConfigValidationError::InvalidYamlRoot(
            "expected a YAML mapping at the document root".to_owned(),
        ));
    };
    if mapping_contains_key(mapping, "infobases") {
        return Err(ConfigValidationError::InfobasesBelongToTheLocalLayer);
    }
    Ok(())
}

/// Который из двух файлов свёртывается: от этого зависят имя в отказе и текст
/// предупреждения.
enum ConfigFile<'a> {
    Project(&'a Path),
    Local,
}

impl ConfigFile<'_> {
    fn name(&self) -> String {
        match self {
            Self::Project(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(DEFAULT_CONFIG_FILE_NAME)
                .to_owned(),
            Self::Local => LOCAL_CONFIG_FILE_NAME.to_owned(),
        }
    }
}

/// Прежний ключ `infobase:` один цикл выпуска читается как `infobases.origin` — в
/// каждом файле отдельно, чтобы проектный файл с прежним ключом и местный слой с
/// новым сливались по полям, как сливались до переименования. Оба ключа в одном
/// файле — отказ: синоним не даёт объявить одну базу дважды.
///
/// Возвращает предупреждение, которое вызывающий покажет пользователю.
fn fold_infobase_synonym(
    root: &mut serde_yaml::Value,
    file: ConfigFile<'_>,
) -> Result<Option<String>, ConfigValidationError> {
    let mapping = root_mapping_mut(root)?;
    let has_old = mapping.contains_key(yaml_key("infobase"));
    let has_new = mapping.contains_key(yaml_key("infobases"));
    if has_old && has_new {
        return Err(ConfigValidationError::InfobaseKeysMixed { file: file.name() });
    }
    let Some(section) = mapping.remove(yaml_key("infobase")) else {
        return Ok(None);
    };
    let mut origin = serde_yaml::Mapping::new();
    origin.insert(yaml_key(DEFAULT_INFOBASE_NAME), section);
    mapping.insert(yaml_key("infobases"), serde_yaml::Value::Mapping(origin));
    let name = file.name();
    let warning = match file {
        ConfigFile::Local => format!(
            "`infobase:` in {name} is a one-cycle synonym for `infobases.{DEFAULT_INFOBASE_NAME}`; rename the key"
        ),
        ConfigFile::Project(_) => format!(
            "`infobase:` in {name} is a one-cycle synonym for `infobases.{DEFAULT_INFOBASE_NAME}`; the section moves to {LOCAL_CONFIG_FILE_NAME}: which infobase a checkout is attached to is known to this machine, not to the project"
        ),
    };
    Ok(Some(warning))
}

/// Выбирает базу запуска и кладёт её в документ как `infobase` и `infobaseName`, чтобы
/// `AppConfig` собрался без сентинела: конфига без выбранной базы не бывает.
fn select_infobase(
    root: &mut serde_yaml::Value,
    selector: &InfobaseSelector,
) -> Result<(), ConfigValidationError> {
    let mapping = root_mapping_mut(root)?;
    // Карта читается на месте: копируется только выбранная секция, не все секции с
    // их паролями.
    let (name, section) = {
        let declared = mapping
            .get(yaml_key("infobases"))
            .and_then(serde_yaml::Value::as_mapping);
        let mut names = Vec::new();
        for key in declared
            .map(serde_yaml::Mapping::keys)
            .into_iter()
            .flatten()
        {
            let name = key.as_str().unwrap_or_default();
            if !is_infobase_name(name) {
                return Err(ConfigValidationError::InfobaseNameInvalid {
                    name: name.to_owned(),
                    pattern: INFOBASE_NAME_PATTERN,
                });
            }
            names.push(name.to_owned());
        }
        let declared_list = if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        };
        let lookup = |name: &str| declared.and_then(|declared| declared.get(yaml_key(name)));
        match selector {
            InfobaseSelector::Default => match lookup(DEFAULT_INFOBASE_NAME) {
                Some(section) => (Some(DEFAULT_INFOBASE_NAME.to_owned()), section.clone()),
                None => {
                    return Err(ConfigValidationError::OriginNotDeclared {
                        declared: declared_list,
                    })
                }
            },
            InfobaseSelector::Name(name) => match lookup(name) {
                Some(section) => (Some(name.clone()), section.clone()),
                None => {
                    return Err(ConfigValidationError::InfobaseNotDeclared {
                        name: name.clone(),
                        declared: declared_list,
                    })
                }
            },
            InfobaseSelector::Connection(connection) => {
                if connection_string_carries_credentials(connection) {
                    return Err(ConfigValidationError::AdHocConnectionCarriesCredentials);
                }
                let mut section = serde_yaml::Mapping::new();
                section.insert(
                    yaml_key("connection"),
                    serde_yaml::Value::String(connection.clone()),
                );
                (None, serde_yaml::Value::Mapping(section))
            }
        }
    };
    mapping.insert(yaml_key("infobase"), section);
    mapping.insert(
        yaml_key("infobaseName"),
        name.map_or(serde_yaml::Value::Null, serde_yaml::Value::String),
    );
    Ok(())
}

/// Реквизиты в строке соединения: `Usr=`/`Pwd=` в объявленной форме, `/N`/`/P` в сырой —
/// и слитно с значением (`/NAdmin /Psecret`), как платформа их принимает. База, названная
/// строкой, учётных данных не несёт — они принадлежат объявленной секции.
fn connection_string_carries_credentials(connection: &str) -> bool {
    let trimmed = connection.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('-') {
        return trimmed.split_whitespace().any(|token| {
            token
                .get(..2)
                .is_some_and(|key| key.eq_ignore_ascii_case("/n") || key.eq_ignore_ascii_case("/p"))
        });
    }
    trimmed.split(';').any(|part| {
        let part = part.trim_start().to_ascii_lowercase();
        part.starts_with("usr=") || part.starts_with("pwd=")
    })
}

pub fn resolve_primary_config_path(config_path: Option<&str>) -> Result<PathBuf, ConfigLoadError> {
    let path = resolve_config_path(config_path)?;
    reject_local_overlay_as_primary_config(&path)?;
    Ok(normalize_windows_verbatim_path(&std::fs::canonicalize(
        &path,
    )?))
}

fn read_yaml_file(path: &Path) -> Result<serde_yaml::Value, ConfigLoadError> {
    let content = std::fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&content)?)
}

/// Ключи `providers.*` документа и имя файла, из которого они пришли.
fn provider_override_keys(root: &serde_yaml::Value, file: &str) -> Vec<(String, String)> {
    root.as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String("providers".to_owned())))
        .and_then(serde_yaml::Value::as_mapping)
        .map(|providers| {
            providers
                .keys()
                .filter_map(serde_yaml::Value::as_str)
                .map(|key| (key.to_owned(), file.to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn reject_local_overlay_as_primary_config(path: &Path) -> Result<(), ConfigLoadError> {
    if path.file_name().and_then(|name| name.to_str()) == Some(LOCAL_CONFIG_FILE_NAME) {
        return Err(ConfigLoadError::LocalOverlayAsPrimaryConfig(
            path.display().to_string(),
        ));
    }
    Ok(())
}

fn reject_local_overlay_keys(root: &serde_yaml::Value) -> Result<(), ConfigLoadError> {
    let Some(mapping) = root.as_mapping() else {
        return Err(ConfigLoadError::ValidationError(
            ConfigValidationError::InvalidYamlRoot(
                "expected a YAML mapping at the document root".to_owned(),
            ),
        ));
    };

    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            return Err(ConfigLoadError::LocalOverlayUnsupportedKey(
                "<non-string>".to_owned(),
            ));
        };
        match key {
            "source-set" => return Err(ConfigLoadError::LocalOverlayForbiddenKey("source-set")),
            "format" => return Err(ConfigLoadError::LocalOverlayForbiddenKey("format")),
            "workPath" | "infobases" | "infobase" | "tools" | "tests" | "mcp" | "providers" => {}
            unsupported => {
                return Err(ConfigLoadError::LocalOverlayUnsupportedKey(
                    unsupported.to_owned(),
                ));
            }
        }
    }

    Ok(())
}

fn default_base_path_to_config_dir(
    root: &mut serde_yaml::Value,
    config_dir: &Path,
) -> Result<(), ConfigLoadError> {
    let Some(mapping) = root.as_mapping_mut() else {
        return Err(ConfigLoadError::ValidationError(
            ConfigValidationError::InvalidYamlRoot(
                "expected a YAML mapping at the document root".to_owned(),
            ),
        ));
    };

    let key = serde_yaml::Value::String("basePath".to_owned());
    if !mapping.contains_key(&key) {
        mapping.insert(
            key,
            serde_yaml::Value::String(config_dir.display().to_string()),
        );
    }

    Ok(())
}

fn merge_yaml_values(base: &mut serde_yaml::Value, overlay: serde_yaml::Value) {
    match (base, overlay) {
        (serde_yaml::Value::Mapping(base), serde_yaml::Value::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// Пути секции базы разрешаются относительно каталога проектного конфига — у каждой
/// объявленной секции, не только у выбранной: местный слой пишут для этой машины.
fn normalize_infobase_paths(infobase: &mut InfobaseConfig, config_dir: &Path) {
    infobase.connection = normalize_connection_string(&infobase.connection, config_dir);
    if let Some(crate::config::model::StandaloneExchangeConfig::Dir { dir }) = infobase
        .standalone
        .as_mut()
        .and_then(|standalone| standalone.exchange.as_mut())
    {
        *dir = normalize_optional_path(dir, config_dir);
    }
    if let Some(web) = infobase.web.as_mut() {
        if let Some(path) = web.dir.as_mut() {
            *path = normalize_optional_path(path, config_dir);
        }
        if let Some(path) = web.conf.as_mut() {
            *path = normalize_optional_path(path, config_dir);
        }
    }
}

fn normalize_config_paths(config: &mut AppConfig, config_dir: &Path) {
    config.base_path = normalize_optional_path(&config.base_path, config_dir);
    config.work_path = normalize_optional_path(&config.work_path, config_dir);
    normalize_infobase_paths(&mut config.infobase, config_dir);
    for infobase in config.infobases.values_mut() {
        normalize_infobase_paths(infobase, config_dir);
    }

    if let Some(path) = config.tools.va.epf_path.as_mut() {
        *path = normalize_optional_path(path, config_dir);
    }
    if let Some(path) = config.tools.platform.path.as_mut() {
        *path = normalize_optional_path(path, config_dir);
    }
    if let Some(extension) = config.tools.client_mcp.extension.as_mut() {
        if let Some(source) = extension.source_mut() {
            source.path = normalize_optional_path(&source.path, config_dir);
        }
        if let Some(artifact) = extension.artifact_mut() {
            artifact.path = normalize_optional_path(&artifact.path, config_dir);
        }
    }
    let va = &mut config.tests.va;
    if let Some(path) = va.params_path.as_mut() {
        *path = normalize_optional_path(path, config_dir);
    }
    for profile in va.profiles.values_mut() {
        if let Some(path) = profile.feature_path.as_mut() {
            *path = normalize_optional_path(path, config_dir);
        }
    }
}

fn reject_legacy_config_keys(root: &serde_yaml::Value) -> Result<(), ConfigValidationError> {
    let Some(mapping) = root.as_mapping() else {
        return Err(ConfigValidationError::InvalidYamlRoot(
            "expected a YAML mapping at the document root".to_owned(),
        ));
    };

    if mapping_contains_key(mapping, "connection") {
        return Err(ConfigValidationError::LegacyTopLevelConnection);
    }

    if mapping_contains_key(mapping, "credentials") {
        return Err(ConfigValidationError::LegacyTopLevelCredentials);
    }

    if mapping_contains_key(mapping, "execution_timeout_seconds") {
        return Err(ConfigValidationError::LegacyTopLevelExecutionTimeoutSeconds);
    }

    if mapping_contains_key(mapping, "builder") {
        return Err(ConfigValidationError::BuilderKeyRemoved);
    }

    if mapping_contains_key(mapping, "execution_timeout") {
        return Err(ConfigValidationError::ExecutionTimeoutKeyRemoved);
    }

    if let Some(mcp) = mapping
        .get(serde_yaml::Value::String("mcp".to_owned()))
        .and_then(serde_yaml::Value::as_mapping)
    {
        if mapping_contains_key(mcp, "client") {
            return Err(ConfigValidationError::LegacyMcpClientConfig);
        }
    }

    if let Some(va) = mapping
        .get(serde_yaml::Value::String("tests".to_owned()))
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|tests| tests.get(serde_yaml::Value::String("va".to_owned())))
        .and_then(serde_yaml::Value::as_mapping)
    {
        if mapping_contains_key(va, "epf_path") {
            return Err(ConfigValidationError::LegacyVanessaEpfPath);
        }
    }

    Ok(())
}

fn mapping_contains_key(mapping: &serde_yaml::Mapping, key: &str) -> bool {
    mapping.contains_key(serde_yaml::Value::String(key.to_owned()))
}

fn normalize_optional_path(path: &Path, config_dir: &Path) -> PathBuf {
    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    };
    normalize_windows_verbatim_path(&normalized)
}

fn normalize_connection_string(connection: &str, config_dir: &Path) -> String {
    let trimmed = connection.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('-') {
        return normalize_raw_connection_args(trimmed, config_dir);
    }

    let mut changed = false;
    let parts: Vec<_> = connection
        .split(';')
        .map(|part| {
            let part = part.trim();
            let lower = part.to_ascii_lowercase();
            if lower.starts_with("file=") {
                let normalized = normalize_connection_file_path(&part[5..], config_dir);
                changed |= normalized != part[5..];
                format!("{}{}", &part[..5], normalized)
            } else {
                part.to_owned()
            }
        })
        .collect();

    if changed {
        parts.join(";")
    } else {
        connection.to_owned()
    }
}

fn normalize_raw_connection_args(connection: &str, config_dir: &Path) -> String {
    let mut args = split_arg_string(connection);
    let mut changed = false;
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index].eq_ignore_ascii_case("/f") || args[index].eq_ignore_ascii_case("-f") {
            let normalized = normalize_connection_file_path(&args[index + 1], config_dir);
            changed |= normalized != args[index + 1];
            args[index + 1] = normalized;
            index += 2;
        } else {
            index += 1;
        }
    }

    if changed {
        join_arg_string(&args)
    } else {
        connection.to_owned()
    }
}

fn normalize_connection_file_path(path: &str, config_dir: &Path) -> String {
    let path = path.trim();
    let path = strip_matching_quotes(path).unwrap_or(path);
    let path = Path::new(path);
    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    };
    normalize_windows_verbatim_path(&normalized)
        .display()
        .to_string()
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }

    let quote = value.as_bytes()[0];
    let last = *value.as_bytes().last()?;
    if (quote == b'\'' || quote == b'"') && quote == last {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

fn split_arg_string(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in raw.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

fn join_arg_string(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(char::is_whitespace) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn resolve_config_path(config_path: Option<&str>) -> Result<PathBuf, ConfigLoadError> {
    if let Some(p) = config_path {
        let path = Path::new(p);
        if !path.exists() {
            return Err(ConfigLoadError::NotFound(p.to_string()));
        }
        return Ok(path.to_path_buf());
    }

    let path = Path::new(DEFAULT_CONFIG_FILE_NAME);
    if path.exists() {
        return Ok(path.to_path_buf());
    }

    Err(ConfigLoadError::NotFound(format!(
        "{DEFAULT_CONFIG_FILE_NAME} (default config file)"
    )))
}

#[cfg(test)]
mod tests {
    use super::{load_config, ConfigLoadError, InfobaseSelector, LOCAL_CONFIG_FILE_NAME};
    use crate::change_detection::partial_load::DEFAULT_PARTIAL_LOAD_THRESHOLD;
    use crate::config::validate::ConfigValidationError;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn write_minimal_project_config(config_dir: &Path, body: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(config_dir.join("src")).expect("src dir");
        let config_path = config_dir.join("v8project.yaml");
        std::fs::write(&config_path, body).expect("write config");
        config_path
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).expect("canonical test path")
    }

    fn minimal_config_without_base_path(extra: &str) -> String {
        format!(
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n{extra}"
        )
    }

    #[test]
    fn load_config_defaults_missing_base_path_to_primary_config_dir() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.base_path, canonical(&config_dir));
        assert_eq!(config.work_path, config.base_path.join("work"));
        assert_eq!(
            config.infobase.connection,
            format!("File={}", config.base_path.join("build/ib").display())
        );
    }

    #[test]
    fn load_config_applies_local_overlay_next_to_primary_config() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("nested").join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\n  user: ProjectUser\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  client_mcp:\n    port: 1111\n  enterprise:\n    additional-launch-keys:\n      - /PROJECT\nmcp:\n  http:\n    path: /project-mcp\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "workPath: local-work\ninfobase:\n  user: LocalUser\n  password: secret\ntools:\n  client_mcp:\n    port: 9874\n  enterprise:\n    additional-launch-keys:\n      - /LOCAL\nmcp:\n  http:\n    path: /local-mcp\n",
        )
        .expect("write local overlay");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.base_path, canonical(&config_dir));
        assert_eq!(config.work_path, config.base_path.join("local-work"));
        assert_eq!(config.infobase.user.as_deref(), Some("LocalUser"));
        assert_eq!(config.infobase.password.as_deref(), Some("secret"));
        assert_eq!(config.tools.client_mcp.port, Some(9874));
        assert_eq!(
            config.tools.enterprise.additional_launch_keys,
            vec!["/LOCAL".to_owned()]
        );
        assert_eq!(config.mcp.http.path, "/local-mcp");
    }

    #[test]
    fn a_local_overlay_replaces_the_allowed_hosts_it_does_not_extend_them() {
        // Решено осознанно: `mcp.http` и так настраивается локальным слоем, а сам
        // слой машинный — кто его пишет, тот перепишет и проектный файл, так что
        // прав это никому не добавляет. Замена списка целиком — общее правило
        // слияния для последовательностей; здесь оно закреплено, потому что для
        // списка про доступ разница между «заменить» и «дополнить» существенна.
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\nmcp:\n  http:\n    allowed_hosts:\n      - from-project\n      - also-project\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "mcp:\n  http:\n    allowed_hosts:\n      - from-local\n",
        )
        .expect("write local overlay");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.mcp.http.allowed_hosts, vec!["from-local".to_owned()]);
    }

    #[test]
    fn load_config_discovers_local_overlay_next_to_explicit_config() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("subproject");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(config_dir.join(LOCAL_CONFIG_FILE_NAME), "workPath: right\n")
            .expect("local overlay");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.work_path, config.base_path.join("right"));
    }

    #[test]
    fn load_config_cli_workdir_override_wins_over_local_overlay() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "workPath: local-work\n",
        )
        .expect("local overlay");

        let config = load_config(
            config_path.to_str(),
            Some("cli-work"),
            &InfobaseSelector::Default,
        )
        .map(|loaded| loaded.config)
        .expect("load config");

        assert_eq!(config.work_path, canonical(&config_dir).join("cli-work"));
    }

    fn load(
        config_path: &Path,
        selector: &InfobaseSelector,
    ) -> Result<super::LoadedConfig, ConfigLoadError> {
        load_config(config_path.to_str(), None, selector)
    }

    /// Проектный `infobase:` и местная карта — одна база `origin`, слитая по полям, и
    /// одно предупреждение: о переезде секции в местный слой.
    #[test]
    fn the_project_synonym_and_the_local_map_merge_into_origin() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "infobases:\n  origin:\n    user: Admin\n  test:\n    connection: \"File=build/test-ib\"\n",
        )
        .expect("local overlay");

        let loaded = load(&config_path, &InfobaseSelector::Default).expect("load config");

        let config = &loaded.config;
        assert_eq!(config.infobase_name.as_deref(), Some("origin"));
        assert_eq!(config.infobase.user.as_deref(), Some("Admin"));
        assert_eq!(
            config.infobase.connection,
            format!("File={}", config.base_path.join("build/ib").display())
        );
        assert_eq!(config.infobases.len(), 2, "{:?}", config.infobases.keys());
        assert_eq!(
            config.infobases["origin"].connection, config.infobase.connection,
            "the selected section is its declared entry"
        );
        assert_eq!(
            config.infobases["test"].connection,
            format!("File={}", config.base_path.join("build/test-ib").display()),
            "every declared section is normalized against the config dir"
        );
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(
            loaded.warnings[0].contains("v8project.yaml"),
            "{:?}",
            loaded.warnings
        );
        assert!(
            loaded.warnings[0].contains("moves to v8project.local.yaml"),
            "{:?}",
            loaded.warnings
        );
    }

    #[test]
    fn a_config_without_the_synonym_loads_without_warnings() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "infobases:\n  origin:\n    connection: \"File=build/ib\"\n",
        )
        .expect("local overlay");

        let loaded = load(&config_path, &InfobaseSelector::Default).expect("load config");

        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.config.infobase_name.as_deref(), Some("origin"));
    }

    #[test]
    fn a_name_selects_its_section_and_a_connection_string_selects_an_ad_hoc_base() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "infobases:\n  origin:\n    connection: \"File=build/ib\"\n  test:\n    connection: \"Srvr=srv;Ref=erp_test\"\n    user: tester\n",
        )
        .expect("local overlay");

        let by_name = load(&config_path, &InfobaseSelector::Name("test".to_owned()))
            .expect("load config")
            .config;
        assert_eq!(by_name.infobase_name.as_deref(), Some("test"));
        assert_eq!(by_name.infobase.connection, "Srvr=srv;Ref=erp_test");
        assert_eq!(by_name.infobase.user.as_deref(), Some("tester"));

        let ad_hoc = load(
            &config_path,
            &InfobaseSelector::Connection("Srvr=ci;Ref=job-417".to_owned()),
        )
        .expect("load config")
        .config;
        assert_eq!(ad_hoc.infobase_name, None);
        assert_eq!(ad_hoc.infobase.connection, "Srvr=ci;Ref=job-417");
        assert_eq!(ad_hoc.infobase.user, None);
        assert_eq!(ad_hoc.infobases.len(), 2, "an ad hoc base is not declared");
    }

    #[test]
    fn a_null_map_in_the_local_layer_is_rejected() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(config_dir.join(LOCAL_CONFIG_FILE_NAME), "infobases: null\n")
            .expect("local overlay");

        let error = load(&config_path, &InfobaseSelector::Default).expect_err("null map");

        assert!(error.to_string().contains("null is not allowed"), "{error}");
    }

    /// Форма невыбранной секции проверяется и называет секцию по имени; среда — нет.
    #[test]
    fn every_declared_section_is_checked_for_form_and_named_when_wrong() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "infobases:\n  prod:\n    connection: \"File=/srv/ib\"\n    dbms:\n      kind: PostgreSQL\n",
        )
        .expect("local overlay");

        let error = load(&config_path, &InfobaseSelector::Default).expect_err("prod is wrong");

        let message = error.to_string();
        assert!(message.contains("infobases.prod:"), "{message}");
        assert!(
            message.contains("infobase.dbms is not allowed for file-based"),
            "{message}"
        );
    }

    #[test]
    fn the_selector_reads_a_name_or_a_connection_string() {
        assert_eq!(InfobaseSelector::from_flag(None), InfobaseSelector::Default);
        assert_eq!(
            InfobaseSelector::from_flag(Some("  ")),
            InfobaseSelector::Default
        );
        assert_eq!(
            InfobaseSelector::from_flag(Some("ci-42")),
            InfobaseSelector::Name("ci-42".to_owned())
        );
        assert_eq!(
            InfobaseSelector::from_flag(Some("Srvr=srv;Ref=erp")),
            InfobaseSelector::Connection("Srvr=srv;Ref=erp".to_owned())
        );
        assert_eq!(
            InfobaseSelector::from_flag(Some("/S srv\\erp")),
            InfobaseSelector::Connection("/S srv\\erp".to_owned())
        );
    }

    #[test]
    fn load_config_rejects_project_identity_keys_in_local_overlay() {
        for key in ["source-set", "format"] {
            let dir = tempdir().expect("tempdir");
            let config_dir = dir.path().join("project");
            let config_path =
                write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
            let value = if key == "source-set" {
                "[]".to_owned()
            } else {
                "DESIGNER".to_owned()
            };
            std::fs::write(
                config_dir.join(LOCAL_CONFIG_FILE_NAME),
                format!("{key}: {value}\n"),
            )
            .expect("local overlay");

            let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
                .map(|loaded| loaded.config)
                .expect_err("forbidden local overlay key");

            assert!(
                error
                    .to_string()
                    .contains(&format!("cannot override project identity key '{key}'")),
                "{error}"
            );
        }
    }

    /// Снятый ключ отклоняется и в оверлее, и с той же подсказкой, что в основном файле.
    #[test]
    fn load_config_names_the_replacement_for_a_builder_key_in_the_local_overlay() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(config_dir.join(LOCAL_CONFIG_FILE_NAME), "builder: IBCMD\n")
            .expect("local overlay");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("builder is removed");

        assert!(
            error.to_string().contains("providers.<operation>"),
            "{error}"
        );
    }

    #[test]
    fn load_config_rejects_unsupported_top_level_keys_in_local_overlay() {
        for key in ["basePath", "build", "unknown"] {
            let dir = tempdir().expect("tempdir");
            let config_dir = dir.path().join("project");
            let config_path =
                write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
            std::fs::write(
                config_dir.join(LOCAL_CONFIG_FILE_NAME),
                format!("{key}: value\n"),
            )
            .expect("local overlay");

            let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
                .map(|loaded| loaded.config)
                .expect_err("unsupported local overlay key");

            assert!(
                error
                    .to_string()
                    .contains(&format!("does not support top-level key '{key}'")),
                "{error}"
            );
        }
    }

    #[test]
    fn load_config_rejects_local_overlay_as_primary_config() {
        let dir = tempdir().expect("tempdir");
        let local_path = dir.path().join(LOCAL_CONFIG_FILE_NAME);
        std::fs::write(&local_path, "workPath: local\n").expect("local overlay");

        let error = load_config(local_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("local overlay cannot be primary");

        assert!(error.to_string().contains("cannot be used as --config"));
    }

    #[test]
    fn load_config_rejects_legacy_keys_from_local_overlay() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "credentials:\n  user: Admin\n",
        )
        .expect("local overlay");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy local key");

        assert!(matches!(
            error,
            ConfigLoadError::ValidationError(ConfigValidationError::LegacyTopLevelCredentials)
        ));
    }

    #[test]
    fn load_config_allows_local_null_for_optional_fields_only() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\n  user: Admin\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "infobase:\n  user: null\n",
        )
        .expect("local overlay");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.infobase.user, None);
    }

    #[test]
    fn a_local_overlay_cannot_unpin_a_declared_host_key() {
        // `null` в локальном слое — общий способ сбросить значение проекта. Для
        // отпечатка ключа это значило бы «снять сверку», причём молча и из файла,
        // которого нет в репозитории. Схема такое запрещает, и загрузчик тоже.
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path = write_minimal_project_config(
            &config_dir,
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  designer_agent:\n    attach: 127.0.0.1:1543\n    base-dir: /tmp/agent\n    host-fingerprint: 'SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU'\n",
        );
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "tools:\n  designer_agent:\n    host-fingerprint: null\n",
        )
        .expect("local overlay");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("a declared fingerprint cannot be reset to null");

        assert!(
            error
                .to_string()
                .contains("local config overlay contains unsupported key or value"),
            "{error}"
        );
    }

    #[test]
    fn load_config_rejects_local_null_for_required_fields() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(config_dir.join(LOCAL_CONFIG_FILE_NAME), "workPath: null\n")
            .expect("local overlay");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("required field cannot be reset to null");

        assert!(
            error
                .to_string()
                .contains("local config overlay contains unsupported key or value"),
            "{error}"
        );
    }

    #[test]
    fn load_config_uses_default_build_settings_when_section_is_omitted() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(
            config.build.partial_load_threshold,
            DEFAULT_PARTIAL_LOAD_THRESHOLD
        );
    }

    #[test]
    fn load_config_rejects_legacy_source_set_purpose_key() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    purpose: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy purpose key");

        assert!(
            error
                .to_string()
                .contains("config contains unsupported key or value"),
            "{error}"
        );
    }

    #[test]
    fn load_config_reads_custom_partial_load_threshold() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nbuild:\n  partialLoadThreshold: 7\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.build.partial_load_threshold, 7);
    }

    #[test]
    fn load_config_reads_test_timeout_from_exact_yaml_key() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntests:\n  execution_timeout_seconds: 17\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.tests.execution_timeout_seconds, 17);
    }

    /// Наложение проходит тот же именной отказ, и раньше общего «ключ не поддержан»:
    /// автор локального файла должен узнать, куда переехал предел, а не что ключ лишний.
    #[test]
    fn local_overlay_refuses_the_retired_execution_timeout_key_by_name_too() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let config_path =
            write_minimal_project_config(&config_dir, &minimal_config_without_base_path(""));
        std::fs::write(
            config_dir.join(LOCAL_CONFIG_FILE_NAME),
            "execution_timeout: 300000\n",
        )
        .expect("local overlay");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("a retired key must be refused in the overlay as well");

        let message = error.to_string();
        assert!(
            message.contains("a command has no deadline"),
            "the overlay must get the named refusal, not the generic one: {message}"
        );
    }

    #[test]
    fn load_config_refuses_the_retired_execution_timeout_key_by_name() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nexecution_timeout: 4321\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("a retired key must be refused, not silently ignored");

        let message = error.to_string();
        assert!(
            message.contains("execution_timeout"),
            "the refusal must name the key the author wrote: {message}"
        );
        assert!(
            message.contains("a command has no deadline"),
            "the refusal must say why the key is gone: {message}"
        );
        assert!(
            message.contains("mcp.execution.admission_timeout_ms"),
            "the refusal must name where a bound still belongs: {message}"
        );
    }

    #[test]
    fn load_config_rejects_top_level_execution_timeout_seconds() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "basePath: {}\nworkPath: {}\nexecution_timeout_seconds: 300\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                base.display(),
                work.display()
            ),
        )
        .expect("write config");

        let err = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("expected legacy key error");

        assert!(matches!(
            err,
            ConfigLoadError::ValidationError(
                ConfigValidationError::LegacyTopLevelExecutionTimeoutSeconds
            )
        ));
    }

    #[test]
    fn load_config_normalizes_vanessa_paths_relative_to_config_dir() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        let features = dir.path().join("cfg").join("features");
        let va = dir.path().join("cfg").join("va");
        std::fs::create_dir_all(&src).expect("src dir");
        std::fs::create_dir_all(&features).expect("features dir");
        std::fs::create_dir_all(&va).expect("va dir");
        std::fs::write(va.join("runner.epf"), "epf").expect("epf");
        std::fs::write(va.join("params.json"), "{}").expect("params");
        let config_dir = dir.path().join("cfg");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        let config_path = config_dir.join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  va:\n    epf_path: va/runner.epf\ntests:\n  va:\n    params_path: va/params.json\n    profile: smoke\n    profiles:\n      smoke:\n        feature_path: features\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: ../base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(
            config.tools.va.epf_path.expect("epf"),
            canonical(&config_dir).join("va/runner.epf")
        );
        assert_eq!(
            config.tests.va.params_path.expect("params"),
            canonical(&config_dir).join("va/params.json")
        );
        assert_eq!(
            config
                .tests
                .va
                .profiles
                .get("smoke")
                .and_then(|profile| profile.feature_path.clone())
                .expect("feature path"),
            canonical(&config_dir).join("features")
        );
    }

    #[test]
    fn load_config_absolutizes_relative_core_paths_from_config_dir() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let base = config_dir.join("sources");
        std::fs::create_dir_all(&base).expect("base dir");
        let config_path = config_dir.join("v8project.yaml");
        std::fs::write(
            &config_path,
            "workPath: build\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\n",
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        let canonical_config_dir = canonical(&config_dir);
        assert_eq!(config.base_path, canonical_config_dir);
        assert_eq!(config.work_path, config.base_path.join("build"));
        assert_eq!(
            config.infobase.connection,
            format!("File={}", config.base_path.join("build/ib").display())
        );
    }

    #[test]
    fn load_config_preserves_server_connection_string() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let base = config_dir.join("sources");
        std::fs::create_dir_all(&base).expect("base dir");
        let config_path = config_dir.join("v8project.yaml");
        std::fs::write(
            &config_path,
            "workPath: build\nformat: DESIGNER\nproviders:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\ninfobase:\n  connection: \"Srvr=cluster:1541;Ref=demo\"\n  dbms:\n    kind: PostgreSQL\n    server: localhost\n    name: demo\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\n",
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.infobase.connection, "Srvr=cluster:1541;Ref=demo");
    }

    #[test]
    fn load_config_rejects_legacy_top_level_connection_key() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "basePath: {}\nworkPath: {}\nformat: DESIGNER\nconnection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                base.display(),
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy connection");

        assert!(error
            .to_string()
            .contains("legacy top-level key 'connection'"));
    }

    #[test]
    fn load_config_rejects_legacy_mcp_client_section() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "basePath: {}\nworkPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nmcp:\n  client:\n    port: 9874\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                base.display(),
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy mcp client");

        assert!(error.to_string().contains("use tools.client_mcp"));
    }

    #[test]
    fn load_config_rejects_legacy_tests_va_epf_path() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "basePath: {}\nworkPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntests:\n  va:\n    epf_path: /tmp/vanessa.epf\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                base.display(),
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy va epf");

        assert!(error.to_string().contains("use tools.va.epf_path"));
    }

    #[test]
    fn load_config_rejects_mixed_new_and_legacy_infobase_keys() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "basePath: {}\nworkPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ncredentials:\n  password: secret\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
                base.display(),
                work.display()
            ),
        )
        .expect("write config");

        let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect_err("reject legacy credentials");

        assert!(error
            .to_string()
            .contains("legacy top-level key 'credentials'"));
    }

    #[test]
    fn load_config_absolutizes_raw_file_connection_from_config_dir() {
        let dir = tempdir().expect("tempdir");
        let config_dir = dir.path().join("project");
        let base = config_dir.join("sources");
        std::fs::create_dir_all(&base).expect("base dir");
        let config_path = config_dir.join("v8project.yaml");
        std::fs::write(
            &config_path,
            "workPath: build\nformat: DESIGNER\ninfobase:\n  connection: '/F \"build/my ib\"'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\n",
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(
            config.v8_connection().file_path(),
            Some(
                canonical(&config_dir)
                    .join("build/my ib")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[test]
    fn load_config_reads_mcp_sections_and_edt_timeouts() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        let extension_src = dir.path().join("exts").join("client-mcp");
        std::fs::create_dir_all(&src).expect("src dir");
        std::fs::create_dir_all(&extension_src).expect("extension dir");
        std::fs::write(
            extension_src.join("Configuration.xml"),
            "<Configuration><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>",
        )
        .expect("extension marker");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nmcp:\n  http:\n    bind_address: 127.0.0.1:4000\n    path: /custom-mcp\n    stateful_sessions: false\n    max_sessions: 12\n    idle_ttl_secs: 45\n    allowed_hosts:\n      - runner\n      - 10.0.0.5\n  execution:\n    max_concurrent_calls: 3\n    shutdown_grace_period_secs: 9\ntools:\n  client_mcp:\n    port: 9874\n    wait_ready_timeout_ms: 4321\n    extension:\n      name: client_mcp\n      source:\n        path: exts/client-mcp\n        format: DESIGNER\n  enterprise:\n    additional-launch-keys:\n      - /TESTMANAGER\n  edt_cli:\n    interactive-mode: true\n    startup_timeout_ms: 1234\n    command_timeout_ms: 5678\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.mcp.http.bind_address, "127.0.0.1:4000");
        assert_eq!(config.mcp.http.path, "/custom-mcp");
        assert!(!config.mcp.http.stateful_sessions);
        assert_eq!(config.mcp.http.max_sessions, 12);
        assert_eq!(config.mcp.http.idle_ttl_secs, 45);
        assert_eq!(
            config.mcp.http.allowed_hosts,
            vec!["runner".to_owned(), "10.0.0.5".to_owned()]
        );
        assert_eq!(config.mcp.execution.max_concurrent_calls, 3);
        assert_eq!(config.mcp.execution.shutdown_grace_period_secs, 9);
        assert_eq!(config.tools.client_mcp.port, Some(9874));
        assert_eq!(config.tools.client_mcp.wait_ready_timeout_ms, Some(4321));
        assert_eq!(
            config.tools.enterprise.additional_launch_keys,
            vec!["/TESTMANAGER".to_owned()]
        );
        assert!(config.tools.edt_cli.interactive_mode);
        assert_eq!(config.tools.edt_cli.startup_timeout_ms, 1234);
        assert_eq!(config.tools.edt_cli.command_timeout_ms, 5678);
        let extension = config
            .tools
            .client_mcp
            .extension
            .expect("client mcp extension");
        assert_eq!(extension.name, "client_mcp");
        assert_eq!(
            extension.source().expect("source").path,
            canonical(dir.path()).join("exts").join("client-mcp")
        );
    }

    #[test]
    fn load_config_rejects_client_mcp_extension_with_multiple_or_missing_inputs() {
        for (case, extension_body) in [
            (
                "both",
                "      name: client_mcp\n      source:\n        path: ext\n      artifact:\n        path: ext.cfe\n",
            ),
            ("neither", "      name: client_mcp\n"),
        ] {
            let dir = tempdir().expect("tempdir");
            let base = dir.path().join("base");
            let work = dir.path().join("work");
            let src = base.join("src");
            std::fs::create_dir_all(&src).expect("src dir");
            let config_path = dir.path().join(format!("{case}.yaml"));
            std::fs::write(
                &config_path,
                format!(
                    "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  client_mcp:\n    extension:\n{extension_body}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                    work.display()
                ),
            )
            .expect("write config");

            let error = load_config(config_path.to_str(), None, &InfobaseSelector::Default).map(|loaded| loaded.config)
                .expect_err("invalid extension input should fail");

            assert!(
                error
                    .to_string()
                    .contains("must specify exactly one of source or artifact"),
                "{error}"
            );
        }
    }

    #[test]
    fn load_config_accepts_enterprise_additional_launch_keys() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");

        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  enterprise:\n    additional-launch-keys:\n      - /TESTMANAGER\n      - /TCUser\n      - ci-user\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");
        assert_eq!(
            config.tools.enterprise.additional_launch_keys,
            vec![
                "/TESTMANAGER".to_owned(),
                "/TCUser".to_owned(),
                "ci-user".to_owned()
            ]
        );
    }

    #[test]
    fn load_config_rejects_removed_config_aliases() {
        let cases = [
            ("executionTimeout", "executionTimeout: 300000\n"),
            ("execution_timeout_ms", "execution_timeout_ms: 300000\n"),
            (
                "edt-cli",
                "tools:\n  edt-cli:\n    startup_timeout_ms: 2222\n",
            ),
            (
                "additional_launch_keys",
                "tools:\n  enterprise:\n    additional_launch_keys:\n      - /TESTMANAGER\n",
            ),
            (
                "additionalLaunchKeys",
                "tools:\n  enterprise:\n    additionalLaunchKeys:\n      - /TESTMANAGER\n",
            ),
            (
                "startup-timeout-ms",
                "tools:\n  edt_cli:\n    startup-timeout-ms: 2222\n",
            ),
            (
                "command-timeout-ms",
                "tools:\n  edt_cli:\n    command-timeout-ms: 3333\n",
            ),
        ];

        for (name, extra_yaml) in cases {
            let dir = tempdir().expect("tempdir");
            let base = dir.path().join("base");
            let work = dir.path().join("work");
            let src = base.join("src");
            std::fs::create_dir_all(&src).expect("src dir");
            let config_path = dir.path().join(format!("{name}.yaml"));
            std::fs::write(
                &config_path,
                format!(
                    "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\n{extra_yaml}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                    work.display()
                ),
            )
            .expect("write config");

            load_config(config_path.to_str(), None, &InfobaseSelector::Default)
                .map(|loaded| loaded.config)
                .expect_err(&format!("{name} alias must be rejected"));
        }
    }

    #[test]
    fn load_config_uses_mcp_and_edt_timeout_defaults_when_sections_are_omitted() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.mcp.http.bind_address, "127.0.0.1:3000");
        assert_eq!(config.mcp.http.path, "/mcp");
        assert!(config.mcp.http.stateful_sessions);
        assert_eq!(config.mcp.http.max_sessions, 64);
        assert_eq!(config.mcp.http.idle_ttl_secs, 900);
        assert!(config.mcp.http.allowed_hosts.is_empty());
        assert_eq!(config.mcp.execution.max_concurrent_calls, 1);
        assert_eq!(config.mcp.execution.shutdown_grace_period_secs, 30);
        assert_eq!(config.tools.edt_cli.startup_timeout_ms, 300_000);
        assert_eq!(config.tools.edt_cli.command_timeout_ms, 300_000);
    }

    #[test]
    fn load_config_accepts_canonical_edt_timeout_keys() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  edt_cli:\n    startup_timeout_ms: 2222\n    command_timeout_ms: 3333\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(config.tools.edt_cli.startup_timeout_ms, 2222);
        assert_eq!(config.tools.edt_cli.command_timeout_ms, 3333);
    }

    #[test]
    fn load_config_reads_edt_version_hint_fields() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  platform:\n    version: 8.3.27.1859\n  edt_cli:\n    path: 1c-edt-2025.2.3\n    version: 1c-edt-2025.2.3\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert_eq!(
            config.tools.platform.version.as_deref(),
            Some("8.3.27.1859")
        );
        assert_eq!(
            config.tools.edt_cli.path.as_deref(),
            Some(std::path::Path::new("1c-edt-2025.2.3"))
        );
        assert_eq!(
            config.tools.edt_cli.version.as_deref(),
            Some("1c-edt-2025.2.3")
        );
    }

    #[test]
    fn load_config_defaults_platform_strict_to_false() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");

        assert!(!config.tools.platform.strict);
    }

    #[test]
    fn load_config_accepts_strict_platform_without_path() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  platform:\n    strict: true\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("strict without path");

        assert!(config.tools.platform.strict);
        assert!(config.tools.platform.path.is_none());
    }

    #[test]
    fn load_config_normalizes_relative_platform_path_against_config_directory() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let src = base.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: \"File=/tmp/ib\"\ntools:\n  platform:\n    path: platform/bin\n    strict: false\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                work.display()
            ),
        )
        .expect("write config");

        let config = load_config(config_path.to_str(), None, &InfobaseSelector::Default)
            .map(|loaded| loaded.config)
            .expect("load config");
        let config_dir = std::fs::canonicalize(dir.path()).expect("canonical config dir");

        assert_eq!(
            config.tools.platform.path.as_deref(),
            Some(config_dir.join("platform/bin").as_path())
        );
    }
}
