use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use super::ensure_platform_success;
use crate::platform::edt::EdtDsl;
use crate::platform::result::PlatformCommandResult;
use crate::support::error::AppError;
use crate::support::fs::{ensure_dir, remove_path_if_exists};
use crate::support::path::{filesystem_object_identity, stable_path_identity};

pub(super) struct ConvertWorkspace<'a> {
    primary: EdtDsl<'a>,
    last_path: PathBuf,
    open: &'a dyn Fn(PathBuf) -> Result<EdtDsl<'a>, AppError>,
}

impl<'a> ConvertWorkspace<'a> {
    pub(super) fn new(
        primary: EdtDsl<'a>,
        open: &'a dyn Fn(PathBuf) -> Result<EdtDsl<'a>, AppError>,
    ) -> Self {
        let last_path = primary.workspace().to_path_buf();
        Self {
            primary,
            last_path,
            open,
        }
    }

    pub(super) fn primary(&self) -> &EdtDsl<'_> {
        &self.primary
    }

    pub(super) fn last_path(&self) -> &Path {
        &self.last_path
    }

    pub(super) fn export(
        &mut self,
        source_set: &str,
        source: &Path,
        target: &Path,
    ) -> Result<(), AppError> {
        self.last_path = self.primary.workspace().to_path_buf();
        let first = self.primary.export_project_path(source, target)?;
        let Some(registered) = registered_path(&self.primary, &first)? else {
            return ensure_platform_success(source_set, "edt-to-designer", &first);
        };

        if let Some(error) = live_collision(&registered, source) {
            return Err(error);
        }
        let Some(stale) = missing_convert_stage(&registered) else {
            return ensure_platform_success(source_set, "edt-to-designer", &first);
        };

        // A stable sibling preserves the live registrations made by previous recoveries.
        // Start in the primary area every time so its live projects still take precedence.
        let fallback_path = self.primary.workspace().with_file_name(format!(
            "edt-workspace-recovery-{}",
            stable_path_identity(&stale),
        ));
        let fallback = (self.open)(fallback_path.clone())?;
        remove_path_if_exists(target)
            .and_then(|()| ensure_dir(target))
            .map_err(|error| {
                AppError::Runtime(format!(
                    "failed to reset convert export staging directory '{}': {error}",
                    target.display()
                ))
            })?;
        self.last_path = fallback_path;
        let result = fallback.export_project_path(source, target)?;
        if let Some(error) =
            registered_path(&fallback, &result)?.and_then(|path| live_collision(&path, source))
        {
            return Err(error);
        }
        ensure_platform_success(source_set, "edt-to-designer", &result)
    }
}

fn registered_path(
    dsl: &EdtDsl<'_>,
    export: &PlatformCommandResult,
) -> Result<Option<PathBuf>, AppError> {
    match dsl.conflicting_project_path(export).map_err(AppError::from) {
        Ok(path) => Ok(path),
        Err(error) if error.cancellation().is_some() => Err(error),
        Err(error) => {
            tracing::debug!(%error, "could not inspect conflicting EDT registration");
            Ok(None)
        }
    }
}

fn live_collision(registered: &Path, source: &Path) -> Option<AppError> {
    let registered_identity = filesystem_object_identity(registered).ok()?;
    let source_identity = filesystem_object_identity(source).ok()?;
    (registered_identity != source_identity).then(|| {
        AppError::Platform(format!(
            "EDT project name collision: source '{}' conflicts with registered project '{}'",
            source.display(),
            registered.display()
        ))
    })
}

fn missing_convert_stage(project: &Path) -> Option<PathBuf> {
    if project
        .components()
        .any(|part| part == Component::ParentDir)
    {
        return None;
    }
    let stage = project.parent()?;
    let name = stage.file_name()?.to_str()?;
    let run_id = name
        .strip_prefix(".convert-stage-")
        .or_else(|| name.strip_prefix(".convert-base-project-"))?;
    let (pid, time) = run_id.split_once('-')?;
    if pid.parse::<u32>().ok()? == 0
        || time.len() != 16
        || !time.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    // Require the output parent itself to be reachable. A missing mount, dangling
    // symlink, access error, or still-existing stage is insufficient evidence.
    let parent = std::fs::canonicalize(stage.parent()?).ok()?;
    for path in [stage, project] {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return None,
        }
    }
    Some(parent.join(name).join(project.file_name()?))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, ProcessRunner,
        SpawnResult, WorkGiven,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext, InterruptionSafetyClass};

    #[derive(Default)]
    struct Edt {
        registrations: RefCell<std::collections::HashMap<PathBuf, PathBuf>>,
    }

    impl ProcessRunner for Edt {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            let workspace = PathBuf::from(&request.args[1]);
            let mut registrations = self.registrations.borrow_mut();
            let registered = registrations.get(&workspace);
            let mut result = ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            };
            match request.args[3].as_str() {
                "project-info" => {
                    assert_eq!(request.args[4], "configuration");
                    result.stdout = format!(
                        "Name                configuration\nLocation            {}\nStarted             false\n",
                        registered.expect("registered project").display()
                    );
                }
                "export" => {
                    let source = PathBuf::from(&request.args[5]);
                    let output = Path::new(&request.args[7]);
                    if registered.is_some_and(|path| *path != source) {
                        result.exit_code = 204;
                        result.stderr =
                            "Other project with name \"configuration\" already exists in workspace."
                                .to_owned();
                        fs::write(output.join("partial.xml"), "incomplete").unwrap();
                    } else {
                        assert!(!output.join("partial.xml").exists());
                        fs::write(output.join("Configuration.xml"), "<Configuration/>").unwrap();
                        registrations.insert(workspace, source);
                    }
                }
                command => panic!("unexpected EDT command: {command}"),
            }
            Ok(result)
        }

        fn spawn(&self, _: &ProcessRequest, _: &WorkGiven) -> Result<SpawnResult, ProcessError> {
            panic!("convert in one-shot mode")
        }
    }

    #[test]
    fn recovery_requires_a_missing_stage_under_a_reachable_output_parent() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let stage = root.join(".convert-stage-42-18e710bce20d9ff8");
        let project = stage.join("configuration");
        assert!(missing_convert_stage(&project).is_some());
        assert!(missing_convert_stage(
            &root
                .join("moved")
                .join(stage.file_name().unwrap())
                .join("configuration")
        )
        .is_none());
        assert!(missing_convert_stage(&root.join("source/configuration")).is_none());
        assert!(missing_convert_stage(
            &stage.join("../.convert-stage-42-18e710bce20d9ff8/configuration")
        )
        .is_none());
        fs::create_dir(&stage).unwrap();
        assert!(missing_convert_stage(&project).is_none());
    }

    #[test]
    fn stale_stage_recovers_but_live_projects_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let primary = root.join("edt-workspace");
        let stale = root.join(".convert-stage-42-18e710bce20d9ff8/configuration");
        let first = root.join("first/configuration");
        let second = root.join("second/configuration");
        let output = root.join("output");
        for source in [&first, &second] {
            fs::create_dir_all(source).unwrap();
            fs::write(source.join(".project"), "original").unwrap();
        }
        fs::create_dir(&output).unwrap();
        let edt = Edt::default();
        edt.registrations
            .borrow_mut()
            .insert(primary.clone(), stale.clone());
        let context = ExecutionContext::cli(CommandName::Convert);
        let open = |path| {
            Ok(EdtDsl::new(
                PathBuf::from("1cedtcli"),
                path,
                &edt,
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            ))
        };
        let convert = || ConvertWorkspace::new(open(primary.clone()).unwrap(), &open);

        let mut first_export = convert();
        first_export.export("main", &first, &output).unwrap();
        let recovered = first_export.last_path.clone();
        assert_ne!(recovered, primary);
        assert!(output.join("Configuration.xml").is_file());
        let mut second_export = convert();
        second_export.export("main", &first, &output).unwrap();
        assert_eq!(second_export.last_path, recovered);
        let mut live_collision = convert();
        let failure = live_collision
            .export("main", &second, &output)
            .unwrap_err()
            .to_string();
        assert!(failure.contains(&first.display().to_string()), "{failure}");
        assert!(failure.contains(&second.display().to_string()), "{failure}");
        assert_eq!(live_collision.last_path, recovered);
        assert_eq!(edt.registrations.borrow().get(&primary), Some(&stale));
        for source in [first, second] {
            assert_eq!(
                fs::read_to_string(source.join(".project")).unwrap(),
                "original"
            );
        }
    }
}
