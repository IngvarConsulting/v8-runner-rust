use std::path::Path;

use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose};
use crate::support::edt_project;
use crate::support::error::AppError;
use crate::support::source_descriptor::SourceDescriptorPurpose;

/// Extension name passed to platform commands (`-Extension` / `--name`).
///
/// The configured source-set name is the extension identity for both Designer and EDT.
/// EDT `.project` name is a workspace/project identity and must not override it.
pub fn platform_extension_name(source_set: &SourceSetConfig) -> &str {
    debug_assert_eq!(source_set.purpose, SourceSetPurpose::Extension);
    source_set.name.as_str()
}

/// Ключ, по которому имя расширения сравнивается с другим: 1С регистр в именах не
/// различает, поэтому `old` в проекте — то же расширение, что `Old` в базе. Одно правило на
/// все сопоставления набора с составом базы (`pull --all`, `download` без набора).
pub fn extension_name_key(name: &str) -> String {
    name.to_lowercase()
}

/// Имя расширения, которое называют исходники набора: `Name` в `Configuration.xml`
/// (у EDT — в `src/Configuration/Configuration.mdo`); `None`, пока описания нет.
///
/// `upload --ref` использует это имя для набора расширения. Другие операции пока
/// сопоставляют наборы с составом базы по имени набора (#218) и используют имя
/// из исходников при проверке расхождений.
pub fn source_extension_name(
    format: SourceFormat,
    root: &Path,
    required_purpose: Option<SourceDescriptorPurpose>,
) -> Result<Option<String>, AppError> {
    let marker = match format {
        SourceFormat::Designer => root.join("Configuration.xml"),
        SourceFormat::Edt => edt_project::ordinary_root_marker_path(root),
    };
    if !marker.is_file() {
        return Ok(None);
    }
    Ok(
        crate::use_cases::config_init::read_configuration_logical_name(&marker, required_purpose)?
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
    )
}

pub(super) fn resolve_upload_extension_ref(
    config: &AppConfig,
    reference: String,
) -> Result<String, AppError> {
    let Some(source_set) = config.source_sets.iter().find(|source_set| {
        source_set.purpose == SourceSetPurpose::Extension && source_set.name == reference
    }) else {
        return Ok(reference);
    };

    let root = source_set.root_in(&config.base_path);
    source_extension_name(
        config.format,
        &root,
        Some(SourceDescriptorPurpose::Extension),
    )?
    .filter(|name| crate::platform::extension_inventory::is_extension_identifier(name))
    .ok_or_else(|| {
        AppError::Validation(format!(
            "--ref '{reference}' requires an extension Configuration.xml with a valid name at '{}'",
            root.join("Configuration.xml").display(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::platform_extension_name;
    use crate::config::model::{SourceSetConfig, SourceSetPurpose};
    use std::path::PathBuf;

    #[test]
    fn platform_extension_name_uses_source_set_name() {
        let source_set = SourceSetConfig {
            name: "SalesAddon".to_owned(),
            purpose: SourceSetPurpose::Extension,
            path: PathBuf::from("extensions/sales-project"),
        };

        assert_eq!(platform_extension_name(&source_set), "SalesAddon");
    }
}
