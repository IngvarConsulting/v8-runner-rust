use std::fs;
use std::path::Path;

use crate::platform::designer::DesignerDsl;
use crate::platform::result::PlatformCommandResult;
use crate::support::error::AppError;
use crate::support::path::normalize_windows_verbatim_path;
use crate::support::source_descriptor::{
    extract_designer_configuration_version, SourceDescriptorParseError,
};
use crate::support::temp::private_temp_dir;

pub(super) fn read_settings(path: &Path) -> Result<Vec<u8>, AppError> {
    let invalid = |reason: &str| {
        AppError::Validation(format!(
            "invalid --settings file '{}': {reason}",
            path.display()
        ))
    };
    let bytes = fs::read(path).map_err(|error| invalid(&error.to_string()))?;
    crate::parsers::merge_settings::validate(&bytes).map_err(|error| invalid(&error))?;
    Ok(bytes)
}

/// The validated content is the content Designer reads, even if the input file changes
/// while Designer is reading the baseline version.
pub(super) fn apply(
    work_path: &Path,
    designer: &DesignerDsl<'_>,
    package: &Path,
    settings_xml: &[u8],
) -> Result<PlatformCommandResult, AppError> {
    let error =
        |error| AppError::Runtime(format!("cannot stage validated update settings: {error}"));
    let temp = private_temp_dir(work_path, "upload-settings-").map_err(error)?;
    let settings = temp.path().join("settings.xml");
    fs::write(&settings, settings_xml).map_err(error)?;
    let result = designer.update_cfg(package, &normalize_windows_verbatim_path(&settings));
    if let Err(error) = temp.close() {
        tracing::warn!(%error, "could not remove the validated update settings");
    }
    Ok(result?)
}

/// Read only the main configuration descriptor. Each read uses an empty, private export
/// directory so that a previous successful read cannot confirm a later incomplete export.
pub(super) fn read_working_version(
    work_path: &Path,
    designer: &DesignerDsl<'_>,
) -> Result<String, AppError> {
    let workspace_error = |error| {
        AppError::Runtime(format!(
            "cannot prepare configuration version read: {error}"
        ))
    };
    let temp = private_temp_dir(work_path, "upload-version-").map_err(workspace_error)?;
    let list = temp.path().join("objects.txt");
    fs::write(&list, "Configuration\n").map_err(workspace_error)?;
    let output = temp.path().join("export");
    fs::create_dir(&output).map_err(workspace_error)?;
    let result = designer.dump_config_to_files_partial(
        &normalize_windows_verbatim_path(&output),
        &normalize_windows_verbatim_path(&list),
        None,
    )?;
    if let Err(code) = result.process.outcome() {
        let mut message =
            format!("reading the working configuration version exited with code {code}");
        if let Some(path) = result.platform_log_path {
            message.push_str(&format!("; see {}", path.display()));
        }
        return Err(AppError::Platform(message));
    }
    let descriptor = fs::read_to_string(output.join("Configuration.xml")).map_err(|error| {
        AppError::Platform(format!("cannot read exported Configuration.xml: {error}"))
    })?;
    let version = extract_designer_configuration_version(&descriptor)
        .map_err(|error| {
            let detail = match error {
                SourceDescriptorParseError::Xml(detail) => detail,
                SourceDescriptorParseError::UnexpectedEof => "unexpected end of XML".to_owned(),
            };
            AppError::Platform(format!("invalid exported Configuration.xml: {detail}"))
        })?
        .filter(|version| !version.is_empty())
        .ok_or_else(|| AppError::Platform("the working configuration has no version".into()))?;
    temp.close().map_err(workspace_error)?;
    Ok(version)
}
