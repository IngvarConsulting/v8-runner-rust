use std::path::Path;

use crate::config::model::{SourceFormat, SourceSetConfig, SourceSetPurpose};
use crate::support::edt_project;
use crate::support::error::AppError;

/// Extension name passed to platform commands (`-Extension` / `--name`).
///
/// The configured source-set name is the extension identity for both Designer and EDT.
/// EDT `.project` name is a workspace/project identity and must not override it.
pub fn platform_extension_name(source_set: &SourceSetConfig) -> &str {
    debug_assert_eq!(source_set.purpose, SourceSetPurpose::Extension);
    source_set.name.as_str()
}

/// Имя расширения, которое называют исходники набора: `Name` в `Configuration.xml`
/// (у EDT — в `src/Configuration/Configuration.mdo`); `None`, пока описания нет.
///
/// Платформе раннер называет расширение по имени набора ([`platform_extension_name`]), а
/// сайт обещает имя набора псевдонимом (#218). Пока обещание не выполнено, это имя служит
/// только сверке: набор, исходники которого называют другое расширение, не должен молча
/// считаться набором ни того, ни другого.
pub fn source_extension_name(
    format: SourceFormat,
    root: &Path,
) -> Result<Option<String>, AppError> {
    let marker = match format {
        SourceFormat::Designer => root.join("Configuration.xml"),
        SourceFormat::Edt => edt_project::ordinary_root_marker_path(root),
    };
    if !marker.is_file() {
        return Ok(None);
    }
    Ok(
        crate::use_cases::config_init::read_configuration_logical_name(&marker)?
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
    )
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
