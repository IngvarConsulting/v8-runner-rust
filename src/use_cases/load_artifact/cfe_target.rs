use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::capability::Provider;
use crate::platform::process::ProcessRunner;
use crate::support::error::AppError;
use crate::support::temp::{private_temp_dir, PrivateTempDir};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::ensure_succeeded;
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::throwaway_infobase::{Builder, FileInfobase};

const PROBE_EXTENSION: &str = "UploadPackageProbe";

/// Keeps the checked bytes available until the real Designer load finishes.
pub(super) struct CheckedCfe {
    workspace: Option<PrivateTempDir>,
    package: PathBuf,
}

impl CheckedCfe {
    pub(super) fn path(&self) -> &Path {
        &self.package
    }
}

impl Drop for CheckedCfe {
    fn drop(&mut self) {
        if let Some(workspace) = self.workspace.take() {
            if let Err(error) = workspace.close() {
                tracing::warn!(%error, package = %self.package.display(), "could not remove the upload package check workspace");
            }
        }
    }
}

pub(super) fn check(
    context: &ExecutionContext,
    work_path: &Path,
    designer: &Path,
    runner: &dyn ProcessRunner,
    artifact: &Path,
    requested: &str,
) -> Result<CheckedCfe, AppError> {
    if let Some(error) = interruption_before_safe_point(context, "CFE package check") {
        return Err(error);
    }
    let stage_error = |error| AppError::Runtime(format!("cannot stage CFE for upload: {error}"));
    let workspace = private_temp_dir(work_path, "upload-cfe-").map_err(stage_error)?;
    let root = workspace.path().to_owned();
    let checked = CheckedCfe {
        workspace: Some(workspace),
        package: root.join("package.cfe"),
    };
    fs::copy(artifact, checked.path()).map_err(stage_error)?;

    let name = read_package_name(context, designer, runner, &root, checked.path())
        .map_err(|error| error.with_context("cannot establish the CFE name before upload"))?;
    if name != requested {
        return Err(AppError::Validation(format!(
            "CFE contains extension '{name}', but --ref resolves to '{requested}'; use the exact extension name"
        )));
    }
    if let Some(error) =
        interruption_before_safe_point(context, "CFE load into the target infobase")
    {
        return Err(error);
    }
    Ok(checked)
}

fn read_package_name(
    context: &ExecutionContext,
    designer: &Path,
    runner: &dyn ProcessRunner,
    root: &Path,
    package: &Path,
) -> Result<String, AppError> {
    let base = FileInfobase::new(
        root.to_owned(),
        Builder {
            provider: Provider::Designer,
            binary: designer.to_owned(),
        },
    );
    let safety = InterruptionSafetyClass::GracefulThenKill;
    base.create(context, runner, safety)?;
    let dsl = base.designer(context, runner, Some(root.join("load.log")), safety);
    // LoadCfg registers the temporary alias. UpdateDBCfg would replace it with the CFE's
    // name, which the working descriptor already exposes without applying the extension.
    let mut loaded = dsl.load_cfg(package, Some(PROBE_EXTENSION))?;
    loaded.platform_log_path = None;
    ensure_succeeded("scratch /LoadCfg", "extension", PROBE_EXTENSION, &loaded)?;

    let list = root.join("objects.txt");
    let output = root.join("export");
    let prepare_error =
        |error| AppError::Runtime(format!("cannot prepare CFE descriptor export: {error}"));
    fs::write(&list, "Configuration\n").map_err(prepare_error)?;
    fs::create_dir(&output).map_err(prepare_error)?;
    let dsl = base.designer(context, runner, Some(root.join("export.log")), safety);
    let mut exported = dsl.dump_config_to_files_partial(&output, &list, Some(PROBE_EXTENSION))?;
    exported.platform_log_path = None;
    ensure_succeeded(
        "scratch /DumpConfigToFiles",
        "extension",
        PROBE_EXTENSION,
        &exported,
    )?;
    crate::use_cases::config_init::read_configuration_logical_name(
        &output.join("Configuration.xml"),
    )?
    .filter(|name| !name.is_empty())
    .ok_or_else(|| AppError::InvalidOutput("exported CFE descriptor has no Name".into()))
}
