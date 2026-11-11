use std::num::NonZeroI32;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::syntax::SyntaxCheckStatus;
use crate::platform::connection::{ClusterInfobaseCreation, V8Connection};
use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessRunner,
};
use crate::platform::result::PlatformCommandResult;
use crate::platform::secrets::mask_text;
use crate::platform::session_list::SessionTerminate;

#[derive(Debug, Error)]
pub enum DesignerError {
    /// Строка подключения не той формы, что нужна команде: `CREATEINFOBASE` файловой базы
    /// ждёт `File=`, базы в кластере — `Srvr=…;Ref=…`.
    #[error("CREATEINFOBASE requires {0}")]
    ConnectionForm(&'static str),

    #[error("failed to execute designer process: {0}")]
    Spawn(ProcessError),

    #[error("failed to remove previous designer /Out log '{path}': {source}")]
    StaleLogCleanup {
        path: PathBuf,
        source: std::io::Error,
    },
}

const SYNTAX_ISSUES_EXIT_CODE: NonZeroI32 = NonZeroI32::new(101).unwrap();

/// Вердикт проверки Конфигуратора — `/CheckConfig` и `/CheckModules` — по коду выхода: ноль —
/// чисто, 101 — замечания найдены, любой другой — сбой инструмента. Других входов у вердикта
/// нет: текст журнала его не меняет.
pub fn syntax_check_status(exit_code: i32) -> SyntaxCheckStatus {
    match exit_code {
        0 => SyntaxCheckStatus::Clean,
        code if code == SYNTAX_ISSUES_EXIT_CODE.get() => SyntaxCheckStatus::IssuesFound,
        _ => SyntaxCheckStatus::ToolFailed,
    }
}

/// Исходы с однозначным кодом проверки Конфигуратора. У сбоя единого кода нет,
/// а при превью команда не выполнялась.
pub(crate) fn syntax_check_outcome(status: SyntaxCheckStatus) -> Option<Result<(), NonZeroI32>> {
    match status {
        SyntaxCheckStatus::Clean => Some(Ok(())),
        SyntaxCheckStatus::IssuesFound => Some(Err(SYNTAX_ISSUES_EXIT_CODE)),
        SyntaxCheckStatus::ToolFailed | SyntaxCheckStatus::Planned => None,
    }
}

/// Low-level DSL for invoking `1cv8` in `DESIGNER` mode.
pub struct DesignerDsl<'a> {
    binary: PathBuf,
    connection: V8Connection,
    runner: &'a dyn ProcessRunner,
    /// Optional file where Designer should write its `/Out` log.
    log_file: Option<PathBuf>,
    execution_policy: ProcessExecutionPolicy,
}

impl<'a> DesignerDsl<'a> {
    /// Create a new Designer DSL bound to one executable path and runner.
    ///
    /// The policy is required: it carries the command's interrupt and its work mark, and a
    /// DSL on a policy of its own would lose both.
    pub fn new(
        binary: PathBuf,
        connection: V8Connection,
        runner: &'a dyn ProcessRunner,
        log_file: Option<PathBuf>,
        execution_policy: ProcessExecutionPolicy,
    ) -> Self {
        Self {
            binary,
            connection,
            runner,
            log_file,
            execution_policy,
        }
    }

    /// `/LoadConfigFromFiles <dir> [-Extension <name>]` без `-updateConfigDumpInfo`: файл
    /// версий раннер держит в своей памяти, а не в каталоге исходников, и загрузка его не
    /// пишет (#646): ключ стоил загрузке на КА 1,8 с.
    pub fn load_config_from_files_full(
        &self,
        source_dir: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/LoadConfigFromFiles".to_owned());
        args.push(source_dir.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/LoadConfigFromFiles <dir> -partial -listFile <list_file> [-Extension <name>]`, тоже
    /// без `-updateConfigDumpInfo`.
    pub fn load_config_from_files_partial(
        &self,
        source_dir: &Path,
        list_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/LoadConfigFromFiles".to_owned());
        args.push(source_dir.display().to_string());
        args.push("-partial".to_owned());
        args.push("-listFile".to_owned());
        args.push(list_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/UpdateDBCfg [-Extension <name>] -Dynamic-`: динамическое обновление запрещено
    /// всегда — без ключа платформа его разрешает (`INV.USE-CASES.A-DB-UPDATE-NEVER-GOES-DYNAMIC`).
    pub fn update_db_cfg(
        &self,
        extension: Option<&str>,
        sessions: SessionTerminate<'_>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/UpdateDBCfg".to_owned());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        args.push("-Dynamic-".to_owned());
        if let SessionTerminate::Force { .. } = sessions {
            args.push("-SessionTerminate".to_owned());
            args.push("force".to_owned());
        }
        self.run(&args)
    }

    /// `/RollbackCfg [-Extension <name>]`: основная конфигурация (или расширение) возвращается
    /// к конфигурации базы данных. Одна команда — одна цель; пустой откат по ответу не
    /// отличить от настоящего (замер 08.10.2026, 8.3.27.2074).
    pub fn rollback_cfg(
        &self,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/RollbackCfg".to_owned());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/LoadCfg <file> [-Extension <name>]`
    pub fn load_cfg(
        &self,
        artifact_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/LoadCfg".to_owned());
        args.push(artifact_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/MergeCfg <file> -Settings <settings> [-Extension <name>]`
    pub fn merge_cfg(
        &self,
        artifact_file: &Path,
        settings_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/MergeCfg".to_owned());
        args.push(artifact_file.display().to_string());
        args.push("-Settings".to_owned());
        args.push(settings_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/UpdateCfg <package> -Settings <settings>`.
    pub fn update_cfg(
        &self,
        package: &Path,
        settings: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        self.run(&self.update_cfg_args(package, settings))
    }

    pub fn preview_update_cfg(&self, package: &Path, settings: &Path) -> String {
        crate::platform::secrets::render_masked_command(
            &self.binary,
            &self.update_cfg_args(package, settings),
        )
    }

    fn update_cfg_args(&self, package: &Path, settings: &Path) -> Vec<String> {
        let mut args = self.base_args();
        args.extend([
            "/UpdateCfg".to_owned(),
            package.display().to_string(),
            "-Settings".to_owned(),
            settings.display().to_string(),
        ]);
        args
    }

    /// `/CompareCfg ... -ReportType Brief -ReportFormat txt -ReportFile <file>`
    pub fn compare_cfg(
        &self,
        first_configuration_type: &str,
        first_name: Option<&str>,
        second_configuration_type: &str,
        second_name: Option<&str>,
        report_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.compare_args(
            first_configuration_type,
            first_name,
            second_configuration_type,
            second_name,
        );
        args.extend(["-ReportType".to_owned(), "Brief".to_owned()]);
        self.run_compare(args, report_file)
    }

    /// Full text report comparing the main configuration with the named target.
    pub(crate) fn compare_full_report(
        &self,
        against: &crate::domain::diff::DiffAgainst,
        report_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        use crate::domain::diff::DiffAgainst;

        let (configuration_type, name) = match against {
            DiffAgainst::Database => ("DBConfiguration", None),
            DiffAgainst::Vendor(name) => ("VendorConfiguration", Some(name.as_str())),
            DiffAgainst::File(_) => ("File", None),
        };
        let mut args = self.compare_args("MainConfiguration", None, configuration_type, name);
        if let DiffAgainst::File(file) = against {
            args.push("-SecondFile".to_owned());
            args.push(file.display().to_string());
        }
        args.extend(
            [
                "-ReportType",
                "Full",
                "-IncludeChangedObjects",
                "-IncludeAddedObjects",
                "-IncludeDeletedObjects",
            ]
            .map(str::to_owned),
        );
        self.run_compare(args, report_file)
    }

    fn compare_args(
        &self,
        first_configuration_type: &str,
        first_name: Option<&str>,
        second_configuration_type: &str,
        second_name: Option<&str>,
    ) -> Vec<String> {
        let mut args = self.base_args();
        args.push("/CompareCfg".to_owned());
        args.push("-FirstConfigurationType".to_owned());
        args.push(first_configuration_type.to_owned());
        if let Some(first_name) = first_name {
            args.push("-FirstName".to_owned());
            args.push(first_name.to_owned());
        }
        args.push("-SecondConfigurationType".to_owned());
        args.push(second_configuration_type.to_owned());
        if let Some(second_name) = second_name {
            args.push("-SecondName".to_owned());
            args.push(second_name.to_owned());
        }
        args
    }

    fn run_compare(
        &self,
        mut args: Vec<String>,
        report_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        args.push("-ReportFormat".to_owned());
        args.push("txt".to_owned());
        args.push("-ReportFile".to_owned());
        args.push(report_file.display().to_string());
        self.run(&args)
    }

    /// `CREATEINFOBASE <connection-string>`
    pub fn create_infobase(&self) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = vec!["CREATEINFOBASE".to_owned()];
        let connection =
            self.connection
                .create_infobase_arg()
                .ok_or(DesignerError::ConnectionForm(
                    "a file-based connection File=…",
                ))?;
        args.push(connection);
        self.run_raw(&args)
    }

    /// `CREATEINFOBASE <клиент-серверная строка> /DisableStartupDialogs`: регистрация базы в
    /// кластере и база данных в СУБД одной командой (замер #181, 8.5.4.1878). Без
    /// `/DisableStartupDialogs` при заполненном списке администраторов кластера и без `SUsr`
    /// клиент спросил бы пароль окном. Путь `/Out`, если задан, должен быть закрытым временным
    /// файлом: платформа может повторить строку соединения с `DBPwd` и `SPwd`.
    pub fn create_cluster_infobase(
        &self,
        creation: &ClusterInfobaseCreation<'_>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let connection = self
            .connection
            .create_cluster_infobase_arg(creation)
            .ok_or(DesignerError::ConnectionForm(
                "a cluster connection Srvr=…;Ref=…",
            ))?;
        let mut args = vec![
            "CREATEINFOBASE".to_owned(),
            connection,
            "/DisableStartupDialogs".to_owned(),
        ];
        if let Some(log_file) = &self.log_file {
            args.push("/Out".to_owned());
            args.push(log_file.display().to_string());
        }
        let mut result = self.run_raw(&args)?;
        let passwords = creation.passwords();
        let connection = args[1].as_str();
        let masked_connection = mask_text(connection, &passwords);
        // Mask an echoed connection as a separate value: its last field can be a
        // password, and its range must not swallow the following diagnostic.
        let mask_output = |output: &str| {
            output
                .split(connection)
                .map(|part| mask_text(part, &passwords))
                .collect::<Vec<_>>()
                .join(&masked_connection)
        };
        result.process.stdout = mask_output(&result.process.stdout);
        result.process.stderr = mask_output(&result.process.stderr);
        result.platform_log = result.platform_log.as_deref().map(mask_output);
        Ok(result)
    }

    /// `/DumpConfigToFiles <dir> -update -getChanges <file> [-Extension <name>]`: что
    /// выгрузка по изменившемуся выгрузила бы, для `diff`. Каталог не трогается, список
    /// пишется в `list_file` (замер #173).
    pub fn dump_changes_list(
        &self,
        target_dir: &Path,
        list_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpConfigToFiles".to_owned());
        args.push(target_dir.display().to_string());
        args.push("-update".to_owned());
        args.push("-getChanges".to_owned());
        args.push(list_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/DumpConfigToFiles <dir> [-Extension <name>]`
    pub fn dump_config_to_files(
        &self,
        target_dir: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpConfigToFiles".to_owned());
        args.push(target_dir.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/DumpConfigToFiles <dir> -update -configDumpInfoForChanges <file> [-Extension <name>]`:
    /// в каталог, которого ещё нет, — только изменённое относительно описи `version_file` и
    /// новая полная опись; после удаления элемента — всё (замер «-configDumpInfoForChanges», #646).
    pub fn dump_config_to_files_for_changes(
        &self,
        target_dir: &Path,
        version_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpConfigToFiles".to_owned());
        args.push(target_dir.display().to_string());
        args.push("-update".to_owned());
        args.push("-configDumpInfoForChanges".to_owned());
        args.push(version_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/GetConfigGenerationID [-Extension <name>]`: токен поколения из файла `/Out`.
    ///
    /// Ответ без токена — неудачный выход, файл `/Out` не прочитан или в нём нет строки из
    /// сорока шестнадцатеричных знаков — это отсутствие ответа, `None`: так и у базы в
    /// кластере, где формат ответа Конфигуратора не замерен.
    pub fn config_generation_id(
        &self,
        extension: Option<&str>,
    ) -> Result<Option<String>, DesignerError> {
        if self.log_file.is_none() {
            return Ok(None);
        }
        let mut args = self.base_args();
        args.push("/GetConfigGenerationID".to_owned());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        let result = self.run(&args)?;
        if result.process.exit_code != 0 {
            return Ok(None);
        }
        Ok(result
            .platform_log
            .as_deref()
            .and_then(crate::platform::generation::generation_token))
    }

    /// `/DumpCfg <file> [-Extension <name>]`
    pub fn dump_cfg(
        &self,
        target_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpCfg".to_owned());
        args.push(target_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/DumpDBCfg <file> [-Extension <name>]`
    pub fn dump_db_cfg(
        &self,
        target_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpDBCfg".to_owned());
        args.push(target_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/DumpDBCfgList -AllExtensions`: имена установленных расширений базы данных, по
    /// одному на строку, в `/Out` (замер #187). Без расширений — пустой `/Out` и `rc=0`.
    pub fn dump_db_cfg_list_all_extensions(&self) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpDBCfgList".to_owned());
        args.push("-AllExtensions".to_owned());
        self.run(&args)
    }

    /// `/DumpIB <file>`
    pub fn dump_infobase(
        &self,
        target_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpIB".to_owned());
        args.push(target_file.display().to_string());
        // The caller formats the failure with the credentials of the transfer or copy.
        self.run_raw(&args)
    }

    /// `/RestoreIB <file>`
    ///
    /// Loads infobase data from a DT file. The platform creates the file infobase
    /// when it is absent and replaces its data when it is present, without asking;
    /// the caller owns that decision.
    pub fn restore_infobase(
        &self,
        source_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/RestoreIB".to_owned());
        args.push(source_file.display().to_string());
        self.run_raw(&args)
    }

    /// `/DumpExternalDataProcessorOrReportToFiles <root-xml> <binary>`
    pub fn dump_external_data_processor_or_report_to_files(
        &self,
        binary_file: &Path,
        root_xml_path: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpExternalDataProcessorOrReportToFiles".to_owned());
        args.push(root_xml_path.display().to_string());
        args.push(binary_file.display().to_string());
        self.run(&args)
    }

    /// `/LoadExternalDataProcessorOrReportFromFiles <root-xml> <binary>`
    pub fn load_external_data_processor_or_report_from_files(
        &self,
        root_xml_path: &Path,
        binary_file: &Path,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/LoadExternalDataProcessorOrReportFromFiles".to_owned());
        args.push(root_xml_path.display().to_string());
        args.push(binary_file.display().to_string());
        self.run(&args)
    }

    /// `/DumpConfigToFiles <dir> -partial -listFile <list_file> [-Extension <name>]`
    pub fn dump_config_to_files_partial(
        &self,
        target_dir: &Path,
        list_file: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/DumpConfigToFiles".to_owned());
        args.push(target_dir.display().to_string());
        args.push("-partial".to_owned());
        args.push("-listFile".to_owned());
        args.push(list_file.display().to_string());
        if let Some(extension) = extension {
            args.push("-Extension".to_owned());
            args.push(extension.to_owned());
        }
        self.run(&args)
    }

    /// `/CheckConfig [-ThinClient] [-Server] ...`
    pub fn check_config(&self, flags: &[&str]) -> Result<PlatformCommandResult, DesignerError> {
        let mut args = self.base_args();
        args.push("/CheckConfig".to_owned());
        args.extend(flags.iter().map(|flag| (*flag).to_owned()));
        // The validation parser needs the original report, including on a nonzero exit.
        self.run_raw(&args)
    }

    fn base_args(&self) -> Vec<String> {
        let mut args = vec![
            "DESIGNER".to_owned(),
            "/DisableStartupDialogs".to_owned(),
            "/DisableStartupMessages".to_owned(),
        ];
        args.extend(self.connection.args());
        if let Some(log_file) = &self.log_file {
            args.push("/Out".to_owned());
            args.push(log_file.display().to_string());
            args.push("-NoTruncate".to_owned());
        }
        args
    }

    /// Preserve machine replies and mask failed runs before their diagnostics leave the adapter.
    fn run(&self, args: &[String]) -> Result<PlatformCommandResult, DesignerError> {
        let mut result = self.run_raw(args)?;
        if let Some(password) = self
            .connection
            .password
            .as_deref()
            .filter(|password| !password.is_empty() && result.process.outcome().is_err())
        {
            let secrets = &[password];
            result.process.stdout = mask_text(&result.process.stdout, secrets);
            result.process.stderr = mask_text(&result.process.stderr, secrets);
            if let Some(log) = &mut result.platform_log {
                *log = mask_text(log, secrets);
            }
        }
        Ok(result)
    }

    fn run_raw(&self, args: &[String]) -> Result<PlatformCommandResult, DesignerError> {
        if let Some(log_file) = &self.log_file {
            match std::fs::remove_file(log_file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(DesignerError::StaleLogCleanup {
                        path: log_file.clone(),
                        source,
                    });
                }
            }
        }
        let process = self
            .runner
            .run_with_policy(
                &ProcessRequest {
                    program: self.binary.clone(),
                    args: args.to_vec(),
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                    windows_verbatim_args: Vec::new(),
                },
                &self.execution_policy,
            )
            .map_err(DesignerError::Spawn)?;

        let (platform_log_path, platform_log, platform_log_read_error) =
            if let Some(path) = &self.log_file {
                match crate::support::fs::read_platform_log(path) {
                    Ok(contents) => (Some(path.clone()), Some(contents), None),
                    Err(error) => (
                        Some(path.clone()),
                        None,
                        Some(format!(
                            "failed to read designer /Out log '{}': {error}",
                            path.display()
                        )),
                    ),
                }
            } else {
                (None, None, None)
            };

        Ok(PlatformCommandResult {
            process,
            platform_log_path,
            platform_log,
            platform_log_read_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{syntax_check_outcome, syntax_check_status, DesignerDsl};
    use crate::domain::syntax::SyntaxCheckStatus;
    use crate::platform::connection::V8Connection;
    use crate::platform::process::{ProcessExecutionPolicy, ProcessExecutor, ProcessRunner};
    use crate::platform::session_list::SessionTerminate;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[derive(Default)]
    struct CapturingRunner(std::cell::RefCell<Vec<crate::platform::process::ProcessRequest>>);

    impl ProcessRunner for CapturingRunner {
        fn run_with_policy(
            &self,
            request: &crate::platform::process::ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<crate::platform::process::ProcessResult, crate::platform::process::ProcessError>
        {
            // Записано то, что получит процесс: аргументы после единственного владельца
            // приставки `\\?\`
            // (`INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX`).
            let mut seen = request.clone();
            seen.args = crate::support::path::child_line(&request.args, None)
                .map_err(
                    |source| crate::platform::process::ProcessError::UnexpressiblePath {
                        cmd: String::new(),
                        source,
                    },
                )?
                .args
                .into_iter()
                .map(std::borrow::Cow::into_owned)
                .collect();
            self.0.borrow_mut().push(seen);
            Ok(crate::platform::process::ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            })
        }

        fn spawn(
            &self,
            _request: &crate::platform::process::ProcessRequest,
            _work: &crate::platform::process::WorkGiven,
        ) -> Result<crate::platform::process::SpawnResult, crate::platform::process::ProcessError>
        {
            panic!("Designer exports must run synchronously")
        }
    }

    /// Каждое `/UpdateDBCfg` запрещает динамическое обновление — у основной конфигурации и
    /// у расширения (`INV.USE-CASES.A-DB-UPDATE-NEVER-GOES-DYNAMIC`).
    #[test]
    fn update_db_cfg_always_forbids_dynamic_update() {
        let runner = CapturingRunner::default();
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            V8Connection::from_connection_string("File=C:/ib"),
            &runner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.update_db_cfg(None, SessionTerminate::Keep)
            .expect("update the configuration");
        dsl.update_db_cfg(Some("ext"), SessionTerminate::Keep)
            .expect("update the extension");
        dsl.update_db_cfg(None, SessionTerminate::Force { message: "m" })
            .expect("update with the sessions ended");

        let requests = runner.0.borrow();
        assert_eq!(requests.len(), 3);
        for (request, expected) in requests.iter().zip([
            &["/UpdateDBCfg", "-Dynamic-"][..],
            &["/UpdateDBCfg", "-Extension", "ext", "-Dynamic-"][..],
            &["/UpdateDBCfg", "-Dynamic-", "-SessionTerminate", "force"][..],
        ]) {
            let start = request
                .args
                .iter()
                .position(|arg| arg == "/UpdateDBCfg")
                .expect("/UpdateDBCfg");
            assert_eq!(&request.args[start..], expected, "{:?}", request.args);
        }
    }

    #[test]
    fn main_configuration_exports_use_the_measured_designer_commands() {
        let runner = CapturingRunner::default();
        let temp = tempdir().expect("temporary log directory");
        let log = temp.path().join("designer.log");
        let log_argument = log.display().to_string();
        let mut connection = V8Connection::from_connection_string("File=C:/ib");
        connection.user = Some("Администратор".to_owned());
        connection.password = Some(String::new());
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            connection,
            &runner,
            Some(log),
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_config_to_files(Path::new("bdump"), None)
            .expect("dump configuration files");
        dsl.dump_cfg(Path::new("b.cf"), None)
            .expect("dump configuration package");

        let requests = runner.0.borrow();
        assert_eq!(requests.len(), 2);
        for (request, operation, output) in [
            (&requests[0], "/DumpConfigToFiles", "bdump"),
            (&requests[1], "/DumpCfg", "b.cf"),
        ] {
            assert_eq!(
                request.args,
                [
                    "DESIGNER",
                    "/DisableStartupDialogs",
                    "/DisableStartupMessages",
                    "/IBConnectionString",
                    "File=C:/ib",
                    "/N",
                    "Администратор",
                    "/Out",
                    log_argument.as_str(),
                    "-NoTruncate",
                    operation,
                    output,
                ]
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn cf_exports_pass_windows_drive_paths_without_verbatim_prefix() {
        assert_cf_export_target(
            r"\\?\C:\Каталог с пробелами\конфигурация.cf",
            r"C:\Каталог с пробелами\конфигурация.cf",
        );
    }

    #[test]
    fn full_comparison_passes_windows_paths_without_verbatim_prefix() {
        let runner = CapturingRunner::default();
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            V8Connection::from_connection_string("File=C:/ib"),
            &runner,
            None,
            ProcessExecutionPolicy::default(),
        );
        let package =
            crate::domain::diff::DiffAgainst::File(r"\\?\C:\Каталог с пробелами\пакет.cf".into());
        dsl.compare_full_report(
            &package,
            Path::new(r"\\?\UNC\server\share\Каталог с пробелами\отчёт.txt"),
        )
        .expect("compare package");
        dsl.compare_cfg(
            "ExtensionConfiguration",
            Some("SalesAddon"),
            "ExtensionDBConfiguration",
            Some("SalesAddon"),
            Path::new("probe.txt"),
        )
        .expect("upload probe");
        let requests = runner.0.borrow();
        // Вне Windows `\\?\` — допустимая часть имени файла, и путь доходит до процесса как есть.
        let (package, report) = if cfg!(windows) {
            (
                r"C:\Каталог с пробелами\пакет.cf",
                r"\\server\share\Каталог с пробелами\отчёт.txt",
            )
        } else {
            (
                r"\\?\C:\Каталог с пробелами\пакет.cf",
                r"\\?\UNC\server\share\Каталог с пробелами\отчёт.txt",
            )
        };
        assert_eq!(
            &requests[0].args[5..],
            [
                "/CompareCfg",
                "-FirstConfigurationType",
                "MainConfiguration",
                "-SecondConfigurationType",
                "File",
                "-SecondFile",
                package,
                "-ReportType",
                "Full",
                "-IncludeChangedObjects",
                "-IncludeAddedObjects",
                "-IncludeDeletedObjects",
                "-ReportFormat",
                "txt",
                "-ReportFile",
                report,
            ]
        );
        assert_eq!(
            &requests[1].args[5..],
            [
                "/CompareCfg",
                "-FirstConfigurationType",
                "ExtensionConfiguration",
                "-FirstName",
                "SalesAddon",
                "-SecondConfigurationType",
                "ExtensionDBConfiguration",
                "-SecondName",
                "SalesAddon",
                "-ReportType",
                "Brief",
                "-ReportFormat",
                "txt",
                "-ReportFile",
                "probe.txt",
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn cf_exports_pass_windows_unc_paths_without_verbatim_prefix() {
        assert_cf_export_target(
            r"\\?\UNC\server\share\Каталог с пробелами\конфигурация.cf",
            r"\\server\share\Каталог с пробелами\конфигурация.cf",
        );
    }

    #[cfg(windows)]
    #[test]
    fn cf_exports_preserve_regular_windows_targets() {
        assert_cf_export_target(
            r"C:\Каталог с пробелами\конфигурация.cf",
            r"C:\Каталог с пробелами\конфигурация.cf",
        );
    }

    #[cfg(windows)]
    fn assert_cf_export_target(target: &str, expected: &str) {
        let runner = CapturingRunner::default();
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            V8Connection::from_connection_string("File=C:/ib"),
            &runner,
            None,
            ProcessExecutionPolicy::default(),
        );
        dsl.dump_cfg(Path::new(target), Some("Расширение"))
            .expect("dump configuration");
        dsl.dump_db_cfg(Path::new(target), None)
            .expect("dump database configuration");

        let requests = runner.0.borrow();
        assert_eq!(requests.len(), 2);
        let mut mismatches = Vec::new();
        for (request, flag, extension) in [
            (&requests[0], "/DumpCfg", true),
            (&requests[1], "/DumpDBCfg", false),
        ] {
            let mut expected_args = vec![
                "DESIGNER",
                "/DisableStartupDialogs",
                "/DisableStartupMessages",
                "/IBConnectionString",
                "File=C:/ib",
                flag,
                expected,
            ];
            if extension {
                expected_args.extend(["-Extension", "Расширение"]);
            }
            if request.args != expected_args {
                mismatches.push(format!(
                    "{flag}: actual {:?}, expected {:?}",
                    request.args, expected_args
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    #[cfg(windows)]
    #[test]
    fn dt_export_passes_windows_drive_path_without_verbatim_prefix() {
        assert_dt_export_target(
            r"\\?\C:\Каталог с пробелами\база.dt",
            r"C:\Каталог с пробелами\база.dt",
        );
    }

    #[cfg(windows)]
    #[test]
    fn dt_export_passes_windows_unc_path_without_verbatim_prefix() {
        assert_dt_export_target(
            r"\\?\UNC\server\share\Каталог с пробелами\база.dt",
            r"\\server\share\Каталог с пробелами\база.dt",
        );
    }

    #[cfg(windows)]
    #[test]
    fn dt_export_preserves_regular_windows_target() {
        assert_dt_export_target(
            r"C:\Каталог с пробелами\база.dt",
            r"C:\Каталог с пробелами\база.dt",
        );
    }

    #[cfg(windows)]
    fn assert_dt_export_target(target: &str, expected: &str) {
        let runner = CapturingRunner::default();
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            V8Connection::from_connection_string("File=C:/ib"),
            &runner,
            None,
            ProcessExecutionPolicy::default(),
        );
        dsl.dump_infobase(Path::new(target)).expect("dump infobase");

        let requests = runner.0.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].args,
            [
                "DESIGNER",
                "/DisableStartupDialogs",
                "/DisableStartupMessages",
                "/IBConnectionString",
                "File=C:/ib",
                "/DumpIB",
                expected,
            ],
            "DT target passed to Designer",
        );
    }

    #[test]
    fn status_mapping_matches_designer_exit_codes() {
        assert_eq!(syntax_check_status(0), SyntaxCheckStatus::Clean);
        assert_eq!(syntax_check_status(101), SyntaxCheckStatus::IssuesFound);
        assert_eq!(syntax_check_status(1), SyntaxCheckStatus::ToolFailed);
        assert_eq!(syntax_check_outcome(syntax_check_status(0)), Some(Ok(())));
        assert_eq!(
            syntax_check_outcome(syntax_check_status(101)),
            std::num::NonZeroI32::new(101).map(Err)
        );
        assert_eq!(syntax_check_outcome(SyntaxCheckStatus::ToolFailed), None);
        assert_eq!(syntax_check_outcome(SyntaxCheckStatus::Planned), None);
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        let staged = path.with_extension("tmp");
        fs::write(&staged, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(&staged);
        fs::rename(&staged, path).expect("rename script");
    }

    /// Откат называет цель ключом `-Extension` только у расширения; учётные данные базы идут
    /// теми же `/N` и `/P`, что у прочих команд.
    #[cfg(unix)]
    #[test]
    fn rollback_cfg_names_the_extension_only_for_an_extension() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!(
                "printf '%s\\n' \"$@\" >> \"{}\"\nexit 0",
                args_log.display()
            ),
        );
        let runner = ProcessExecutor;
        let mut connection = V8Connection::from_connection_string("File=/tmp/ib");
        connection.user = Some("Admin".to_owned());
        connection.password = Some("secret".to_owned());
        let dsl = DesignerDsl::new(
            script,
            connection,
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.rollback_cfg(None).expect("main");
        dsl.rollback_cfg(Some("Patch")).expect("extension");

        let args = fs::read_to_string(args_log).expect("args");
        let runs: Vec<&str> = args.split("/RollbackCfg\n").collect();
        assert_eq!(runs.len(), 3, "{args}");
        assert!(!runs[1].contains("-Extension"), "{args}");
        assert!(runs[2].starts_with("-Extension\nPatch\n"), "{args}");
        assert!(
            args.contains("/N\nAdmin\n") && args.contains("/P\nsecret\n"),
            "{args}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_run_when_previous_platform_log_cannot_be_removed() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let marker = dir.path().join("designer-ran");
        let log_path = dir.path().join("designer.log");
        fs::create_dir(&log_path).expect("log path directory");
        write_script(&script, &format!("touch \"{}\"\nexit 0", marker.display()));
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            Some(log_path),
            ProcessExecutionPolicy::default(),
        );

        let error = dsl
            .check_config(&[])
            .expect_err("stale log cleanup must fail closed");

        assert!(error
            .to_string()
            .contains("failed to remove previous designer /Out log"));
        assert!(
            !marker.exists(),
            "Designer must not run with a stale /Out log"
        );
    }

    #[cfg(unix)]
    #[test]
    fn returns_none_platform_log_when_out_is_not_requested() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        write_script(&script, "exit 0");
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        let result = dsl.check_config(&[]).expect("check config");

        assert!(result.platform_log_path.is_none());
        assert!(result.platform_log.is_none());
        assert!(result.platform_log_read_error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reads_platform_log_when_out_is_requested() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let log_path = dir.path().join("designer.log");
        write_script(
            &script,
            "while [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"/Out\" ]; then\n    shift\n    printf 'designer log\\n' > \"$1\"\n    break\n  fi\n  shift\ndone\nexit 1",
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            Some(log_path.clone()),
            ProcessExecutionPolicy::default(),
        );

        let result = dsl.check_config(&["-Server"]).expect("check config");

        assert_eq!(result.process.exit_code, 1);
        assert_eq!(result.platform_log_path, Some(log_path));
        assert_eq!(result.platform_log.as_deref(), Some("designer log\n"));
        assert!(result.platform_log_read_error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_process_result_when_platform_log_is_unreadable() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let log_path = dir.path().join("missing").join("designer.log");
        write_script(&script, "exit 101");
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            Some(log_path.clone()),
            ProcessExecutionPolicy::default(),
        );

        let result = dsl.check_config(&["-Server"]).expect("check config");

        assert_eq!(result.process.exit_code, 101);
        assert_eq!(result.platform_log_path, Some(log_path));
        assert!(result.platform_log.is_none());
        assert!(result
            .platform_log_read_error
            .as_deref()
            .expect("read error")
            .contains("failed to read designer /Out log"));
    }

    #[cfg(unix)]
    #[test]
    fn dump_config_to_files_full_does_not_request_incremental_update() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_config_to_files(dir.path().join("out").as_path(), None)
            .expect("dump config");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("/DumpConfigToFiles"));
        assert!(!args.contains("-update"));
        assert!(!args.contains("-updateConfigDumpInfo"));
    }

    #[cfg(unix)]
    #[test]
    fn dump_config_to_files_for_changes_passes_the_runner_copy() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_config_to_files_for_changes(
            dir.path().join("out").as_path(),
            dir.path().join("copy.xml").as_path(),
            Some("ExtName"),
        )
        .expect("dump config incremental");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("/DumpConfigToFiles"));
        assert!(
            args.contains("-update\n-configDumpInfoForChanges\n"),
            "{args}"
        );
        assert!(args.contains("copy.xml"), "{args}");
        assert!(!args.contains("-updateConfigDumpInfo"));
        assert!(args.contains("-Extension"));
        assert!(args.contains("ExtName"));
    }

    #[cfg(unix)]
    #[test]
    fn dump_config_to_files_partial_passes_partial_list_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_config_to_files_partial(
            dir.path().join("out").as_path(),
            dir.path().join("objects.txt").as_path(),
            Some("ExtName"),
        )
        .expect("dump config partial");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("/DumpConfigToFiles"));
        assert!(args.contains("-partial"));
        assert!(args.contains("-listFile"));
        assert!(args.contains("objects.txt"));
        assert!(!args.contains("-updateConfigDumpInfo"));
        assert!(args.contains("-Extension"));
        assert!(args.contains("ExtName"));
    }

    #[cfg(unix)]
    #[test]
    fn create_infobase_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/my ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.create_infobase().expect("create infobase");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("CREATEINFOBASE"));
        assert!(args.contains("File='/tmp/my ib'"));
    }

    #[test]
    fn create_cluster_infobase_passes_the_string_and_requests_the_configured_log() {
        let dir = tempdir().expect("tempdir");
        let log = dir.path().join("out.log");
        let runner = CapturingRunner::default();
        let dsl = DesignerDsl::new(
            "1cv8.exe".into(),
            V8Connection::from_connection_string("Srvr=srv;Ref=demo"),
            &runner,
            Some(log.clone()),
            ProcessExecutionPolicy::default(),
        );

        dsl.create_cluster_infobase(&crate::platform::connection::ClusterInfobaseCreation {
            dbms: "PostgreSQL",
            database_server: "db",
            database_name: "demo",
            database_user: None,
            database_password: None,
            locale: "ru",
            cluster_user: None,
            cluster_password: None,
        })
        .expect("create infobase");

        assert_eq!(
            runner.0.borrow()[0].args,
            [
                "CREATEINFOBASE",
                "Srvr=srv;Ref=demo;DBMS=PostgreSQL;DBSrvr=db;DB=demo;CrSQLDB=Y;Locale=ru;SchJobDn=Y",
                "/DisableStartupDialogs",
                "/Out",
                &log.to_string_lossy(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn dump_cfg_passes_dumpcfg_extension_and_out_arguments() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        let log_path = dir.path().join("designer.log");
        let target = dir.path().join("release.cfe");
        write_script(
            &script,
            &format!(
                "printf '%s\\n' \"$*\" > \"{}\"\nprev=''\nfor arg in \"$@\"; do if [ \"$prev\" = '/Out' ]; then printf 'designer log' > \"$arg\"; fi; prev=\"$arg\"; done\nexit 0",
                args_log.display()
            ),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            Some(log_path.clone()),
            ProcessExecutionPolicy::default(),
        );

        let result = dsl.dump_cfg(&target, Some("SalesAddon")).expect("dump cfg");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("/DumpCfg"));
        assert!(args.contains(&target.display().to_string()));
        assert!(args.contains("-Extension SalesAddon"));
        assert!(args.contains(&format!("/Out {}", log_path.display())));
        assert_eq!(result.platform_log_path, Some(log_path));
    }

    #[cfg(unix)]
    #[test]
    fn dump_db_cfg_builds_expected_args_for_extension() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        let target = dir.path().join("database.cfe");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_db_cfg(&target, Some("SalesAddon"))
            .expect("dump database configuration");

        let args = fs::read_to_string(args_log)
            .expect("args log")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "DESIGNER",
                "/DisableStartupDialogs",
                "/DisableStartupMessages",
                "/IBConnectionString",
                "File=/tmp/ib",
                "/DumpDBCfg",
                target.to_str().expect("utf-8 target"),
                "-Extension",
                "SalesAddon",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn dump_infobase_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        let target = dir.path().join("snapshot.dt");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_infobase(&target).expect("dump infobase");

        let args = fs::read_to_string(args_log)
            .expect("args log")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "DESIGNER",
                "/DisableStartupDialogs",
                "/DisableStartupMessages",
                "/IBConnectionString",
                "File=/tmp/ib",
                "/DumpIB",
                target.to_str().expect("utf-8 target"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_and_merge_cfg_pass_expected_arguments() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$*\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.load_cfg(&dir.path().join("release.cf"), None)
            .expect("load cfg");
        let args = fs::read_to_string(&args_log).expect("args log");
        assert!(args.contains("/LoadCfg"));
        assert!(args.contains("release.cf"));

        dsl.merge_cfg(
            &dir.path().join("release.cfe"),
            &dir.path().join("merge.xml"),
            Some("SalesAddon"),
        )
        .expect("merge cfg");
        let args = fs::read_to_string(&args_log).expect("args log");
        assert!(args.contains("/MergeCfg"));
        assert!(args.contains("release.cfe"));
        assert!(args.contains("-Settings"));
        assert!(args.contains("merge.xml"));
        assert!(args.contains("-Extension SalesAddon"));
    }

    #[cfg(unix)]
    #[test]
    fn compare_cfg_passes_configuration_types_names_and_report_file() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$*\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.compare_cfg(
            "ExtensionConfiguration",
            Some("SalesAddon"),
            "ExtensionDBConfiguration",
            Some("SalesAddon"),
            &dir.path().join("compare.txt"),
        )
        .expect("compare cfg");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("/CompareCfg"));
        assert!(args.contains("-FirstConfigurationType ExtensionConfiguration"));
        assert!(args.contains("-FirstName SalesAddon"));
        assert!(args.contains("-SecondConfigurationType ExtensionDBConfiguration"));
        assert!(args.contains("-SecondName SalesAddon"));
        assert!(args.contains("-ReportType Brief"));
        assert!(args.contains("-ReportFormat txt"));
        assert!(args.contains("compare.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn external_dump_and_load_commands_shape_arguments() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = DesignerDsl::new(
            script,
            V8Connection::from_connection_string("File=/tmp/ib"),
            &runner as &dyn ProcessRunner,
            None,
            ProcessExecutionPolicy::default(),
        );

        dsl.dump_external_data_processor_or_report_to_files(
            &dir.path().join("artifact.epf"),
            &dir.path().join("dump/Artifact.xml"),
        )
        .expect("external dump");
        let args = fs::read_to_string(&args_log)
            .expect("args log")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let dump_pos = args
            .iter()
            .position(|line| line == "/DumpExternalDataProcessorOrReportToFiles")
            .expect("dump arg");
        assert_eq!(
            args[dump_pos + 1],
            dir.path().join("dump/Artifact.xml").display().to_string()
        );
        assert_eq!(
            args[dump_pos + 2],
            dir.path().join("artifact.epf").display().to_string()
        );

        dsl.load_external_data_processor_or_report_from_files(
            &dir.path().join("dump/Artifact.xml"),
            &dir.path().join("artifact.epf"),
        )
        .expect("external load");
        let args = fs::read_to_string(&args_log)
            .expect("args log")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let load_pos = args
            .iter()
            .position(|line| line == "/LoadExternalDataProcessorOrReportFromFiles")
            .expect("load arg");
        assert_eq!(
            args[load_pos + 1],
            dir.path().join("dump/Artifact.xml").display().to_string()
        );
        assert_eq!(
            args[load_pos + 2],
            dir.path().join("artifact.epf").display().to_string()
        );
    }
}
