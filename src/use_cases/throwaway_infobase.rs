//! A throwaway file infobase for unpacking a package, and the shared platform
//! operations for local file infobases owned by the runner.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::model::{AppConfig, SourceSetConfig};
use crate::domain::capability::Provider;
use crate::platform::connection::V8Connection;
use crate::platform::designer::DesignerDsl;
use crate::platform::edt::EdtDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl, IbcmdInfobaseCreateStatus};
use crate::platform::locator::{platform_version_of, UtilityType};
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::{remove_path_if_exists, write_temp_dir_metadata, TempDirKind};
use crate::support::temp::temp_root;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::{collecting_deferrals, interruption_before_safe_point};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::source_inventory::SourceSetInventory;
use crate::use_cases::staged_publication::{cleanup_owned_orphan_files, make_run_id};

/// Корень временных баз под `workPath/temp`.
const ROOT_NAME: &str = "throwaway-infobases";
/// Каталог одной базы — `base-<запуск>`.
const PREFIX: &str = "base-";
/// Чьи это следы: описание каждой базы называет его вместо цели публикации.
const IDENTITY: &str = "v8-runner throwaway infobase";

/// Рабочая область EDT, в которой исходники набора переводятся в XML для временной базы: та
/// же, что у шага сборки `push`, чей перевод здесь и выполняется.
pub(crate) fn edt_workspace(work_path: &Path) -> PathBuf {
    work_path.join("edt-workspace")
}

/// Корень временных баз прогонов под `workPath`.
pub(crate) fn throwaway_root(work_path: &Path) -> std::io::Result<PathBuf> {
    Ok(temp_root(work_path)?.join(ROOT_NAME))
}

/// Исполнитель и его утилита. Процессы запускает переданный в каждый вызов `runner`: база
/// живёт дольше одного вызова, а исполнитель процессов принадлежит вызывающему.
#[derive(Debug, Clone)]
pub(crate) struct Builder {
    pub provider: Provider,
    pub binary: PathBuf,
}

impl Builder {
    pub(crate) fn platform_identity(&self) -> String {
        let utility = if self.provider == Provider::Ibcmd {
            UtilityType::Ibcmd
        } else {
            UtilityType::V8
        };
        match platform_version_of(utility, &self.binary) {
            Some(version) => version.to_string(),
            None => {
                let binary = self
                    .binary
                    .canonicalize()
                    .unwrap_or_else(|_| self.binary.clone());
                let fingerprint = std::fs::metadata(&binary)
                    .ok()
                    .map(|metadata| (metadata.len(), metadata.modified().ok()));
                format!("{}:{}:{fingerprint:?}", self.provider, binary.display())
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct FileInfobase {
    dir: PathBuf,
    builder: Builder,
}

#[derive(Debug)]
pub(crate) struct ThrowawayInfobase {
    base: FileInfobase,
    warnings: Vec<String>,
    removed: bool,
}

impl ThrowawayInfobase {
    /// Убирает брошенные базы прошлых прогонов, затем на безопасной точке создаёт свою.
    ///
    /// Каталог и его описание появляются до запуска исполнителя: оборванное создание
    /// оставляет след, который уборка узнаёт.
    pub(crate) fn create(
        context: &ExecutionContext,
        work_path: &Path,
        builder: Builder,
        runner: &dyn ProcessRunner,
    ) -> Result<Self, AppError> {
        Self::create_after(context, work_path, builder, runner, cleanup_orphans)
    }

    /// [`Self::create`] с уборкой брошенных баз `cleanup`: так проверяется, что неудачная
    /// уборка прогон не останавливает.
    fn create_after(
        context: &ExecutionContext,
        work_path: &Path,
        builder: Builder,
        runner: &dyn ProcessRunner,
        cleanup: impl FnOnce(&Path) -> Result<(), AppError>,
    ) -> Result<Self, AppError> {
        let root = throwaway_root(work_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to prepare the throwaway infobase root: {error}"
            ))
        })?;
        // Уборка брошенных баз прогон не останавливает: базу, которую держит зависший
        // процесс или антивирус, уберёт следующий прогон, а своя база ляжет в свой каталог.
        let warnings = cleanup(&root)
            .err()
            .map(|error| {
                format!(
                    "stale throwaway infobases were not removed: {error}; the next package-to-XML conversion retries"
                )
            })
            .into_iter()
            .collect();
        if let Some(error) = interruption_before_safe_point(context, "throwaway infobase creation")
        {
            return Err(error);
        }
        std::fs::create_dir_all(&root).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the throwaway infobase root '{}': {error}",
                root.display()
            ))
        })?;
        let run_id = make_run_id();
        let dir = root.join(format!("{PREFIX}{run_id}"));
        std::fs::create_dir(&dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the throwaway infobase directory '{}': {error}",
                dir.display()
            ))
        })?;
        let base = Self {
            base: FileInfobase::new(dir, builder),
            warnings,
            removed: false,
        };
        write_temp_dir_metadata(
            &base.base.dir,
            TempDirKind::ThrowawayInfobase,
            &run_id,
            &base.base.dir,
            IDENTITY,
        )
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to describe the throwaway infobase '{}': {error}",
                base.base.dir.display()
            ))
        })?;
        log_live_stage(
            "throwaway infobase",
            "creating a throwaway infobase under workPath",
        );
        let created = base
            .base
            .create(context, runner, InterruptionSafetyClass::GracefulThenKill);
        if let Err(error) = created {
            // Убрать сразу: база, которую не создали, не нужна ни этому прогону, ни уборке.
            // Неудачная уборка — своя или брошенных баз — едет с отказом, а не теряется.
            let warnings = base.close();
            if warnings.is_empty() {
                return Err(error);
            }
            return Err(error.with_context(warnings.join("; ")));
        }
        Ok(base)
    }

    /// Разбирает файл пакета `.cf` или `.cfe` в XML каталога `target_dir`: `ibcmd config
    /// export --file` этой базы, сама база при этом не читается. Исход утилиты не судится:
    /// его проверяет вызывающий. Разбирает пакет только `ibcmd` — строка `convert` матрицы.
    pub(crate) fn export_package(
        &self,
        context: &ExecutionContext,
        runner: &dyn ProcessRunner,
        package_file: &Path,
        target_dir: &Path,
    ) -> Result<PlatformCommandResult, AppError> {
        match self.base.builder.provider {
            Provider::Ibcmd => {
                if let Some(error) = interruption_before_safe_point(context, "package export") {
                    return Err(error);
                }
                log_live_stage("package export", "[ibcmd] exporting the package to XML");
                self.base
                    .ibcmd(context, runner, InterruptionSafetyClass::GracefulThenKill)
                    .config_export_file(package_file, target_dir)
                    .map_err(AppError::from)
            }
            other => Err(crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::Convert,
                other,
            )),
        }
    }

    /// Убирает базу. Неудача — предупреждение: пакет уже собран, а след узнает уборка. К
    /// нему добавляется неудачная уборка брошенных баз при создании.
    pub(crate) fn close(mut self) -> Vec<String> {
        let mut warnings = std::mem::take(&mut self.warnings);
        warnings.extend(self.remove());
        warnings
    }

    fn remove(&mut self) -> Option<String> {
        if self.removed {
            return None;
        }
        self.removed = true;
        let sidecar = crate::support::fs::metadata_sidecar_path(&self.base.dir);
        let removed =
            remove_path_if_exists(&self.base.dir).and_then(|()| remove_path_if_exists(&sidecar));
        removed.err().map(|error| {
            format!(
                "failed to remove the throwaway infobase '{}': {error}; the next package-to-XML conversion removes it once it is stale",
                self.base.dir.display()
            )
        })
    }
}

impl FileInfobase {
    pub(crate) fn new(dir: PathBuf, builder: Builder) -> Self {
        Self { dir, builder }
    }

    pub(crate) fn connection(&self) -> V8Connection {
        V8Connection::from_connection_string(&format!("File={}", self.database_path().display()))
    }

    pub(crate) fn create(
        &self,
        context: &ExecutionContext,
        runner: &dyn ProcessRunner,
        safety: InterruptionSafetyClass,
    ) -> Result<Vec<String>, AppError> {
        collecting_deferrals(|deferrals| match self.builder.provider {
            Provider::Ibcmd => {
                let outcome = self
                    .ibcmd(context, runner, safety)
                    .ensure_infobase_create()?;
                deferrals.note_result("build infobase creation", &outcome.result);
                match outcome.status {
                    IbcmdInfobaseCreateStatus::Created => Ok(()),
                    IbcmdInfobaseCreateStatus::Unconfirmed(error) => Err(AppError::from(error)),
                    IbcmdInfobaseCreateStatus::AlreadyExists
                    | IbcmdInfobaseCreateStatus::Failed => Err(creation_failure(&outcome.result)),
                }
            }
            Provider::Designer => {
                let result = self
                    .designer(context, runner, None, safety)
                    .create_infobase()?;
                deferrals.note_result("build infobase creation", &result);
                result
                    .process
                    .outcome()
                    .map_err(|_| creation_failure(&result))
            }
            other => Err(unsupported(other)),
        })
        .map(|((), warnings)| warnings)
    }

    fn database_path(&self) -> PathBuf {
        self.dir.join("ib")
    }

    fn data_path(&self) -> PathBuf {
        self.dir.join("data")
    }

    pub(crate) fn ibcmd<'r>(
        &self,
        context: &ExecutionContext,
        runner: &'r dyn ProcessRunner,
        safety: InterruptionSafetyClass,
    ) -> IbcmdDsl<'r> {
        IbcmdDsl::new(
            self.builder.binary.clone(),
            IbcmdConnection::File {
                database_path: self.database_path(),
                user: None,
                password: None,
            },
            runner,
            context.process_policy(safety, None),
        )
        .with_data_path(self.data_path())
    }

    pub(crate) fn designer<'r>(
        &self,
        context: &ExecutionContext,
        runner: &'r dyn ProcessRunner,
        log_file: Option<PathBuf>,
        safety: InterruptionSafetyClass,
    ) -> DesignerDsl<'r> {
        DesignerDsl::new(
            self.builder.binary.clone(),
            self.connection(),
            runner,
            log_file,
            context.process_policy(safety, None),
        )
    }

    pub(crate) fn load_full(
        &self,
        context: &ExecutionContext,
        runner: &dyn ProcessRunner,
        source_dir: &Path,
        extension: Option<&str>,
        log_file: Option<PathBuf>,
        safety: InterruptionSafetyClass,
    ) -> Result<PlatformCommandResult, AppError> {
        if let Some(error) = interruption_before_safe_point(context, "sources load") {
            return Err(error);
        }
        match self.builder.provider {
            Provider::Designer => self
                .designer(context, runner, log_file, safety)
                .load_config_from_files_untouched(source_dir, extension)
                .map_err(AppError::from),
            Provider::Ibcmd => self
                .ibcmd(context, runner, safety)
                .config_import_full(source_dir, extension)
                .map_err(AppError::from),
            other => Err(unsupported(other)),
        }
    }

    pub(crate) fn dump_package(
        &self,
        context: &ExecutionContext,
        runner: &dyn ProcessRunner,
        output: &Path,
        extension: Option<&str>,
        log_file: Option<PathBuf>,
    ) -> Result<PlatformCommandResult, AppError> {
        if let Some(error) = interruption_before_safe_point(context, "package dump") {
            return Err(error);
        }
        log_live_stage(
            "package dump",
            "dumping the package from the build infobase",
        );
        match self.builder.provider {
            Provider::Designer => self
                .designer(
                    context,
                    runner,
                    log_file,
                    InterruptionSafetyClass::GracefulThenKill,
                )
                .dump_cfg(output, extension)
                .map_err(AppError::from),
            Provider::Ibcmd => self
                .ibcmd(context, runner, InterruptionSafetyClass::GracefulThenKill)
                .config_save(output, false, extension)
                .map_err(AppError::from),
            other => Err(unsupported(other)),
        }
    }
}

impl Drop for ThrowawayInfobase {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

/// Чем [`edt_sources_to_xml`] переводит исходники EDT.
pub(crate) enum EdtConversion<'s> {
    /// Общая сессия EDT, которую команда уже держит: одноразовый `1cedtcli` упёрся бы в
    /// рабочую область сессии. Предел — у самой сессии (`command_timeout_ms`).
    Session(&'s EdtDsl<'s>),
    /// Одноразовый `1cedtcli` с пределом шага: `None` — без предела, как у `push`.
    OneShot { timeout: Option<Duration> },
}

/// Единственный перевод исходников набора формата EDT в XML каталога `target`: `1cedtcli`
/// шагом сборки `push` (`build_project::execute_edt_export_step`) в рабочей области
/// [`edt_workspace`]. Его зовут база сборки `make` и `convert`
/// и сборка файловой базы проекта EDT у
/// `infobase create`. Чем переводить, задаёт вызывающий ([`EdtConversion`]): `make` и
/// `infobase create` без общей сессии — одноразовым процессом без предела, как `push`,
/// `convert` — с пределом EDT команды (`ExecutionContext::edt_timeout`), `infobase create` при
/// общей сессии — через неё, с её пределом.
/// Ответ — предупреждения шага.
pub(crate) fn edt_sources_to_xml(
    context: &ExecutionContext,
    config: &AppConfig,
    source_set: &SourceSetConfig,
    target: &Path,
    runner: EdtConversion<'_>,
) -> Result<Vec<String>, AppError> {
    let inventory = SourceSetInventory::new(config);
    let edt_context = inventory.edt_context(&source_set.name).ok_or_else(|| {
        AppError::Runtime(format!(
            "missing EDT context for source-set '{}'",
            source_set.name
        ))
    })?;
    let mut utilities = PlatformUtilities::from_config(config);
    let one_shot;
    let edt = match runner {
        EdtConversion::Session(session) => session,
        EdtConversion::OneShot { timeout } => {
            let location = utilities
                .locate(UtilityType::EdtCli)
                .map_err(AppError::from)?;
            one_shot = EdtDsl::new(
                location.path,
                edt_workspace(&config.work_path),
                utilities.runner_for(UtilityType::EdtCli),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            )
            .with_timeout(timeout);
            &one_shot
        }
    };
    log_live_stage("edt export", "[EDT] converting the sources to XML");
    use crate::use_cases::build_project::EdtExportSource;
    let export_source = match context.command() {
        crate::use_cases::context::CommandName::Artifacts
        | crate::use_cases::context::CommandName::Convert => {
            EdtExportSource::ProjectPath(edt_context)
        }
        _ => EdtExportSource::Registered(edt_context),
    };
    crate::use_cases::build_project::execute_edt_export_step(
        context,
        config,
        edt,
        source_set,
        export_source,
        target,
        context.command().as_str(),
    )
}

/// Брошенные базы прошлых прогонов: свои по описанию и имени, старше срока уборки. Чужое и
/// свежее остаётся — сканер один на все временные следы раннера.
fn cleanup_orphans(root: &Path) -> Result<(), AppError> {
    cleanup_owned_orphan_files(&[root.to_path_buf()], root, IDENTITY, &[PREFIX], &[], true)
}

fn creation_failure(result: &PlatformCommandResult) -> AppError {
    let mut details = vec![format!(
        "failed to create the throwaway infobase: exit code {}",
        result.process.exit_code
    )];
    for (name, text) in [
        ("stdout", &result.process.stdout),
        ("stderr", &result.process.stderr),
    ] {
        if !text.trim().is_empty() {
            details.push(format!("{name}: {}", text.trim()));
        }
    }
    AppError::Platform(details.join("; "))
}

fn unsupported(provider: Provider) -> AppError {
    crate::use_cases::unimplemented_provider(crate::domain::capability::Operation::Make, provider)
}

#[cfg(test)]
mod tests {
    use super::{throwaway_root, IDENTITY, PREFIX};
    use crate::support::fs::{
        metadata_sidecar_path, read_temp_dir_metadata, write_temp_dir_metadata, TempDirKind,
    };

    fn stale(dir: &std::path::Path, kind: TempDirKind, identity: &str) {
        std::fs::create_dir_all(dir).expect("dir");
        write_temp_dir_metadata(dir, kind, "run-1", dir, identity).expect("metadata");
        let sidecar = metadata_sidecar_path(dir);
        let mut metadata = read_temp_dir_metadata(dir).expect("read");
        metadata.created_at -= chrono::Duration::days(2);
        std::fs::write(&sidecar, serde_json::to_vec(&metadata).expect("json")).expect("write");
    }

    /// Брошенную базу, которую не убрать (её держит процесс), уборка не превращает в отказ:
    /// своя база создаётся в своём каталоге, а неудача становится предупреждением.
    #[cfg(unix)]
    #[test]
    fn a_stale_base_that_cannot_be_removed_does_not_stop_the_build() {
        use std::os::unix::fs::PermissionsExt;
        let work = tempfile::tempdir().expect("work");
        let designer = work.path().join("1cv8");
        std::fs::write(&designer, "#!/bin/sh\nexit 0\n").expect("script");
        std::fs::set_permissions(&designer, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let runner = crate::platform::process::ProcessExecutor;

        let base = super::ThrowawayInfobase::create_after(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Artifacts,
            ),
            work.path(),
            super::Builder {
                provider: crate::domain::capability::Provider::Designer,
                binary: designer,
            },
            &runner,
            |_| {
                Err(crate::support::error::AppError::Runtime(
                    "failed to remove stale publication temp: locked".to_owned(),
                ))
            },
        )
        .expect("created despite the stale base");

        let dir = base.base.dir.clone();
        assert!(dir.is_dir());
        let warnings = base.close();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("stale throwaway infobases were not removed")),
            "{warnings:?}"
        );
        assert!(!dir.exists());
    }

    /// Неудачная уборка брошенных баз не теряется и тогда, когда своя база не создалась:
    /// отказ называет её.
    #[cfg(unix)]
    #[test]
    fn a_failed_creation_names_the_failed_orphan_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        let work = tempfile::tempdir().expect("work");
        let designer = work.path().join("1cv8");
        std::fs::write(&designer, "#!/bin/sh\nexit 1\n").expect("script");
        std::fs::set_permissions(&designer, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let runner = crate::platform::process::ProcessExecutor;

        let error = super::ThrowawayInfobase::create_after(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Artifacts,
            ),
            work.path(),
            super::Builder {
                provider: crate::domain::capability::Provider::Designer,
                binary: designer,
            },
            &runner,
            |_| {
                Err(crate::support::error::AppError::Runtime(
                    "failed to remove stale publication temp: locked".to_owned(),
                ))
            },
        )
        .expect_err("creation failed");

        let message = error.to_string();
        assert!(
            message.contains("stale throwaway infobases were not removed"),
            "{message}"
        );
        assert!(
            message.contains("failed to create the throwaway infobase"),
            "{message}"
        );
    }

    /// Уборка узнаёт брошенную базу по описанию и имени и убирает её; свежую, чужую и
    /// каталог с другим именем оставляет.
    #[test]
    fn orphan_cleanup_removes_only_stale_own_throwaway_bases() {
        let work = tempfile::tempdir().expect("work");
        let root = throwaway_root(work.path()).expect("root");
        let own = root.join(format!("{PREFIX}run-1"));
        stale(&own, TempDirKind::ThrowawayInfobase, IDENTITY);
        let foreign = root.join(format!("{PREFIX}run-1-foreign"));
        stale(&foreign, TempDirKind::ThrowawayInfobase, "someone else");
        let renamed = root.join("other-run-1");
        stale(&renamed, TempDirKind::ThrowawayInfobase, IDENTITY);
        let fresh = root.join(format!("{PREFIX}run-2"));
        std::fs::create_dir_all(&fresh).expect("fresh");
        write_temp_dir_metadata(
            &fresh,
            TempDirKind::ThrowawayInfobase,
            "run-2",
            &fresh,
            IDENTITY,
        )
        .expect("fresh metadata");

        super::cleanup_orphans(&root).expect("cleanup");

        assert!(!own.exists());
        assert!(!metadata_sidecar_path(&own).exists());
        assert!(foreign.exists());
        assert!(renamed.exists());
        assert!(fresh.exists());
    }
}
