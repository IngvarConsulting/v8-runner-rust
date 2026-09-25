use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::model::InfobaseConfig;
use crate::platform::connection::{file_infobase, name_the_account, V8Connection};
use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessRunner,
};
use crate::platform::result::PlatformCommandResult;

#[derive(Debug, Error)]
pub enum IbcmdError {
    #[error("server-based IBCMD connection requires infobase.dbms.{0}")]
    MissingServerDbmsField(&'static str),

    #[error("failed to execute ibcmd process: {0}")]
    Spawn(ProcessError),
}

/// Connection contract passed to `ibcmd infobase ...` commands.
#[derive(Debug, Clone)]
pub enum IbcmdConnection {
    File {
        database_path: PathBuf,
        user: Option<String>,
        password: Option<String>,
    },
    Server {
        dbms_kind: String,
        database_server: String,
        database_name: String,
        user: Option<String>,
        password: Option<String>,
        database_user: Option<String>,
        database_password: Option<String>,
    },
}

impl IbcmdConnection {
    /// Maps the public `infobase` config contract into `ibcmd` arguments.
    pub fn from_infobase(infobase: &InfobaseConfig) -> Result<Self, IbcmdError> {
        let conn = V8Connection::from_connection_string(&infobase.connection);
        let Some(database_path) = conn.file_path() else {
            let Some(dbms) = infobase.dbms.as_ref() else {
                return Err(IbcmdError::MissingServerDbmsField("kind"));
            };

            return Ok(Self::Server {
                dbms_kind: required_dbms_field("kind", dbms.kind.as_deref())?,
                database_server: required_dbms_field("server", dbms.server.as_deref())?,
                database_name: required_dbms_field("name", dbms.name.as_deref())?,
                user: infobase.user.clone(),
                password: infobase.password.clone(),
                database_user: dbms.user.clone(),
                database_password: dbms.password.clone(),
            });
        };

        Ok(Self::File {
            database_path: PathBuf::from(database_path),
            user: infobase.user.clone(),
            password: infobase.password.clone(),
        })
    }

    /// Names the target infobase and the account without echoing any secret.
    ///
    /// The raw connection string may carry `Pwd=`, so it is never reproduced here; only
    /// the pieces a caller needs to recognise the target are named.
    pub fn describe_target(&self) -> String {
        let (target, user) = match self {
            Self::File {
                database_path,
                user,
                ..
            } => (file_infobase(database_path.display()), user),
            Self::Server {
                dbms_kind,
                database_server,
                database_name,
                user,
                ..
            } => (
                format!("{dbms_kind} infobase '{database_name}' on '{database_server}'"),
                user,
            ),
        };
        name_the_account(&target, user.as_deref())
    }

    #[cfg(test)]
    fn args(&self) -> Vec<String> {
        let mut args = self.infobase_args();
        args.extend(self.auth_args());
        args.extend(self.dbms_auth_args());
        args
    }

    fn infobase_args(&self) -> Vec<String> {
        match self {
            Self::File { database_path, .. } => {
                let mut args = Vec::new();
                // 8.3.20 accepts only this short alias and expects it before nested commands.
                push_option_value(&mut args, "--db-path", database_path.display().to_string());
                args
            }
            Self::Server {
                dbms_kind,
                database_server,
                database_name,
                ..
            } => {
                let mut args = Vec::new();
                push_option_value(&mut args, "--dbms", dbms_kind);
                push_option_value(&mut args, "--database-server", database_server);
                push_option_value(&mut args, "--database-name", database_name);
                args
            }
        }
    }

    fn auth_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let (user, password) = match self {
            Self::File { user, password, .. } | Self::Server { user, password, .. } => {
                (user, password)
            }
        };
        if let Some(user) = user {
            push_option_value(&mut args, "--user", user);
        }
        if let Some(password) = password {
            if !password.is_empty() {
                push_option_value(&mut args, "--password", password);
            }
        }
        args
    }

    fn dbms_auth_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Self::Server {
            database_user,
            database_password,
            ..
        } = self
        {
            if let Some(user) = database_user {
                if !user.trim().is_empty() {
                    push_option_value(&mut args, "--database-user", user);
                }
            }
            if let Some(password) = database_password {
                if !password.is_empty() {
                    push_option_value(&mut args, "--database-password", password);
                }
            }
        }
        args
    }
}

/// Dynamic apply mode supported by `ibcmd config apply`.
#[derive(Debug, Clone, Copy)]
pub enum DynamicUpdateMode {
    Auto,
}

impl DynamicUpdateMode {
    fn as_str(self) -> &'static str {
        match self {
            DynamicUpdateMode::Auto => "auto",
        }
    }
}

/// Result status returned by `ibcmd infobase create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbcmdInfobaseCreateStatus {
    Created,
    AlreadyExists,
    Failed,
}

/// Normalized outcome for infobase creation with the raw platform payload preserved.
#[derive(Debug)]
pub struct IbcmdInfobaseCreateOutcome {
    pub status: IbcmdInfobaseCreateStatus,
    pub result: PlatformCommandResult,
}

/// Low-level DSL for invoking `ibcmd`.
pub struct IbcmdDsl<'a> {
    binary: PathBuf,
    connection: IbcmdConnection,
    runner: &'a dyn ProcessRunner,
    execution_policy: ProcessExecutionPolicy,
    data_path: Option<PathBuf>,
}

impl<'a> IbcmdDsl<'a> {
    /// Creates a new DSL bound to a resolved `ibcmd` binary and target infobase.
    ///
    /// The policy is required: it carries the command's interrupt and its work mark.
    pub fn new(
        binary: PathBuf,
        connection: IbcmdConnection,
        runner: &'a dyn ProcessRunner,
        execution_policy: ProcessExecutionPolicy,
    ) -> Self {
        Self {
            binary,
            connection,
            runner,
            execution_policy,
            data_path: None,
        }
    }

    /// Uses an isolated standalone-server data directory for every IBCMD call.
    pub fn with_data_path(mut self, data_path: PathBuf) -> Self {
        self.data_path = Some(data_path);
        self
    }

    /// Imports a full configuration or extension snapshot into the target infobase.
    pub fn config_import_full(
        &self,
        source_dir: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "import"]);
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push(source_dir.display().to_string());
        self.run(&args)
    }

    /// Ensures the infobase exists, asking the infobase itself what a failed create means.
    ///
    /// `ibcmd infobase create` answers 255 both when the infobase is already registered and
    /// when the path cannot be written (measured on 8.3.27.2074), so its exit code alone does
    /// not separate the benign case. The separation comes from a second structural question
    /// rather than from the complaint's wording (DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES): `config generation-id` answers
    /// zero only when the infobase exists **and** these credentials can read it — a missing
    /// infobase and a wrong user both answer 255. So a create that failed over an infobase we
    /// can still read is "already there", and anything else stays a failure, including the case
    /// where the infobase exists but is not ours to touch.
    pub fn ensure_infobase_create(&self) -> Result<IbcmdInfobaseCreateOutcome, IbcmdError> {
        let args = self.create_infobase_args();
        let result = self.run(&args)?;
        if result.process.exit_code == 0 {
            return Ok(IbcmdInfobaseCreateOutcome {
                status: IbcmdInfobaseCreateStatus::Created,
                result,
            });
        }
        let probe = self.run(&self.authenticated_infobase_args(&["config", "generation-id"]))?;
        let status = if probe.process.exit_code == 0 {
            IbcmdInfobaseCreateStatus::AlreadyExists
        } else {
            IbcmdInfobaseCreateStatus::Failed
        };

        Ok(IbcmdInfobaseCreateOutcome { status, result })
    }

    /// Updates extension security properties in the target infobase.
    pub fn infobase_extension_update_properties(
        &self,
        name: &str,
        safe_mode: bool,
        unsafe_action_protection: bool,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "extension", "update"]);
        push_option_value(&mut args, "--name", name);
        push_option_value(
            &mut args,
            "--safe-mode",
            if safe_mode { "yes" } else { "no" },
        );
        push_option_value(
            &mut args,
            "--unsafe-action-protection",
            if unsafe_action_protection {
                "yes"
            } else {
                "no"
            },
        );
        self.run(&args)
    }

    /// Reads the extension composition of the target infobase.
    ///
    /// `ibcmd config extension list` is the only way to read it: Designer has no batch
    /// key that reports installed extensions.
    pub fn infobase_extension_list(&self) -> Result<PlatformCommandResult, IbcmdError> {
        let args = self.authenticated_infobase_args(&["config", "extension", "list"]);
        self.run(&args)
    }

    /// Reads one extension of the target infobase by name.
    pub fn infobase_extension_info(&self, name: &str) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "extension", "info"]);
        push_option_value(&mut args, "--name", name);
        self.run(&args)
    }

    /// Registers a new extension in the target infobase.
    pub fn infobase_extension_create(
        &self,
        name: &str,
        name_prefix: &str,
        synonym: Option<&str>,
        purpose: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "extension", "create"]);
        push_option_value(&mut args, "--name", name);
        push_option_value(&mut args, "--name-prefix", name_prefix);
        if let Some(synonym) = synonym {
            push_option_value(&mut args, "--synonym", synonym);
        }
        if let Some(purpose) = purpose {
            push_option_value(&mut args, "--purpose", purpose);
        }
        self.run(&args)
    }

    /// Removes one extension from the target infobase.
    pub fn infobase_extension_delete(
        &self,
        name: &str,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "extension", "delete"]);
        push_option_value(&mut args, "--name", name);
        self.run(&args)
    }

    /// Sets extension activity in the target infobase.
    pub fn infobase_extension_set_active(
        &self,
        name: &str,
        active: bool,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "extension", "update"]);
        push_option_value(&mut args, "--name", name);
        push_option_value(&mut args, "--active", if active { "yes" } else { "no" });
        self.run(&args)
    }

    /// Imports a partial file list into the target infobase.
    pub fn config_import_partial(
        &self,
        base_dir: &Path,
        files: &[PathBuf],
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "import", "files"]);
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push("--partial".to_owned());
        push_option_value(&mut args, "--base-dir", base_dir.display().to_string());
        args.extend(files.iter().map(|path| path.display().to_string()));
        self.run(&args)
    }

    /// Applies imported configuration changes to the infobase.
    pub fn config_apply(
        &self,
        extension: Option<&str>,
        dynamic: DynamicUpdateMode,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "apply"]);
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push("--force".to_owned());
        push_option_value(&mut args, "--dynamic", dynamic.as_str());
        self.run(&args)
    }

    /// Exports a full configuration or extension snapshot from the infobase.
    pub fn config_export_full(
        &self,
        target_dir: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "export"]);
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push("--force".to_owned());
        args.push(target_dir.display().to_string());
        self.run(&args)
    }

    /// Saves the working or database configuration to a CF/CFE artifact.
    pub fn config_save(
        &self,
        target_file: &Path,
        database: bool,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "save"]);
        if database {
            args.push("--db".to_owned());
        }
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push(target_file.display().to_string());
        self.run(&args)
    }

    /// Exports a saved CF/CFE file using the `config` mode's target context.
    ///
    /// This is deliberately distinct from `config_export_full`, which reads the
    /// working configuration rather than the saved applied DB snapshot. Platform
    /// 8.3.27 still requires database connection arguments even with `--file`.
    pub fn config_export_file(
        &self,
        source_file: &Path,
        target_dir: &Path,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = vec!["config".to_owned()];
        args.extend(self.base_args());
        args.push("export".to_owned());
        args.extend(self.connection.auth_args());
        args.extend(self.connection.dbms_auth_args());
        args.push(format!("--file={}", source_file.display()));
        args.push(target_dir.display().to_string());
        self.run(&args)
    }

    /// Exports changes in sync mode relative to an existing target directory.
    pub fn config_export_incremental(
        &self,
        target_dir: &Path,
        extension: Option<&str>,
    ) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args = self.authenticated_infobase_args(&["config", "export"]);
        if let Some(extension) = extension {
            push_option_value(&mut args, "--extension", extension);
        }
        args.push("--sync".to_owned());
        args.push(target_dir.display().to_string());
        self.run(&args)
    }

    fn base_args(&self) -> Vec<String> {
        self.connection.infobase_args()
    }

    fn infobase_args(&self, command: &[&str]) -> Vec<String> {
        let mut args = vec!["infobase".to_owned()];
        args.extend(self.base_args());
        args.extend(command.iter().map(|part| (*part).to_owned()));
        args
    }

    fn authenticated_infobase_args(&self, command: &[&str]) -> Vec<String> {
        let mut args = self.infobase_args(command);
        args.extend(self.connection.auth_args());
        args.extend(self.connection.dbms_auth_args());
        args
    }

    fn create_infobase_args(&self) -> Vec<String> {
        let mut args = self.infobase_args(&["create"]);
        if matches!(self.connection, IbcmdConnection::Server { .. }) {
            args.push("--create-database".to_owned());
        }
        args.extend(self.connection.auth_args());
        args.extend(self.connection.dbms_auth_args());
        args
    }

    fn run(&self, args: &[String]) -> Result<PlatformCommandResult, IbcmdError> {
        let mut args_with_data = args.to_vec();
        if let Some(data_path) = &self.data_path {
            args_with_data.insert(1, data_path.display().to_string());
            args_with_data.insert(1, "--data".to_owned());
        }
        let process = self
            .runner
            .run_with_policy(
                &ProcessRequest {
                    program: self.binary.clone(),
                    args: args_with_data,
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &self.execution_policy,
            )
            .map_err(IbcmdError::Spawn)?;

        Ok(PlatformCommandResult {
            process,
            platform_log_path: None,
            platform_log: None,
            platform_log_read_error: None,
        })
    }
}

fn push_option_value(args: &mut Vec<String>, key: &str, value: impl ToString) {
    args.push(key.to_owned());
    args.push(value.to_string());
}

fn required_dbms_field(field: &'static str, value: Option<&str>) -> Result<String, IbcmdError> {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(IbcmdError::MissingServerDbmsField(field)),
    }
}

#[cfg(test)]
mod tests {
    use super::{DynamicUpdateMode, IbcmdConnection, IbcmdDsl, IbcmdInfobaseCreateStatus};
    use crate::config::model::{InfobaseConfig, InfobaseDbmsConfig};
    use crate::platform::process::{ProcessExecutionPolicy, ProcessExecutor, ProcessRunner};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

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
        let mut file = fs::File::create(&staged).expect("create script");
        file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
            .expect("write script");
        file.sync_all().expect("sync script");
        drop(file);
        make_executable(&staged);
        fs::rename(&staged, path).expect("rename script");
    }

    fn file_connection(path: &str) -> IbcmdConnection {
        IbcmdConnection::from_infobase(&InfobaseConfig::file(path)).expect("connection")
    }

    #[test]
    fn ibcmd_connection_from_file_path() {
        let ibcmd = file_connection("File=/tmp/ib");

        assert_eq!(ibcmd.args(), vec!["--db-path", "/tmp/ib"]);
    }

    #[test]
    fn ibcmd_connection_from_server_uses_dbms_contract() {
        let ibcmd = IbcmdConnection::from_infobase(&InfobaseConfig::server(
            "Srvr=demo;Ref=test",
            InfobaseDbmsConfig::new("PostgreSQL", "localhost", "demo")
                .with_credentials(Some("postgres".to_owned()), Some("secret".to_owned())),
        ))
        .expect("connection");

        assert_eq!(
            ibcmd.args(),
            vec![
                "--dbms",
                "PostgreSQL",
                "--database-server",
                "localhost",
                "--database-name",
                "demo",
                "--database-user",
                "postgres",
                "--database-password",
                "secret"
            ]
        );
    }

    #[test]
    fn ibcmd_connection_includes_auth_args() {
        let ibcmd = IbcmdConnection::from_infobase(
            &InfobaseConfig::file("File=/tmp/ib")
                .with_credentials(Some("admin".to_owned()), Some("secret".to_owned())),
        )
        .expect("connection");

        assert_eq!(
            ibcmd.args(),
            vec![
                "--db-path",
                "/tmp/ib",
                "--user",
                "admin",
                "--password",
                "secret"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_import_full_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_import_full(dir.path(), Some("Ext"))
            .expect("import");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("config"));
        assert!(args.contains("import"));
        assert!(args.contains("infobase\n--db-path\n/ib\nconfig\nimport"));
        assert!(args.contains("--extension\nExt"));
    }

    #[cfg(unix)]
    #[test]
    fn config_import_partial_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );
        let files = vec![PathBuf::from("Catalogs/Items.xml")];

        dsl.config_import_partial(dir.path(), &files, None)
            .expect("import");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("import"));
        assert!(args.contains("files"));
        assert!(args.contains("--partial"));
        assert!(args.contains("--base-dir\n"));
        assert!(args.contains("Catalogs/Items.xml"));
    }

    #[cfg(unix)]
    #[test]
    fn config_apply_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_apply(None, DynamicUpdateMode::Auto)
            .expect("apply");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("apply"));
        assert!(args.contains("--force"));
        assert!(args.contains("--dynamic\nauto"));
    }

    #[cfg(unix)]
    #[test]
    fn config_export_full_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_export_full(dir.path(), None).expect("export");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("export"));
        assert!(args.contains("--force"));
    }

    #[cfg(unix)]
    #[test]
    fn config_export_file_uses_config_mode_and_the_target_context() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        let source = dir.path().join("database.cfe");
        let target = dir.path().join("xml");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let dsl = IbcmdDsl::new(
            script,
            file_connection("File=/ib"),
            &runner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_export_file(&source, &target)
            .expect("offline export");

        let args = fs::read_to_string(args_log).expect("args");
        let args = args.lines().collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "config",
                "--db-path",
                "/ib",
                "export",
                format!("--file={}", source.display()).as_str(),
                target.to_str().expect("utf8 target"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_save_builds_expected_args_for_database_extension() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        let target = dir.path().join("database.cfe");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_save(&target, true, Some("SalesAddon"))
            .expect("save database extension");

        let args = fs::read_to_string(args_log)
            .expect("args")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "infobase",
                "--db-path",
                "/ib",
                "config",
                "save",
                "--db",
                "--extension",
                "SalesAddon",
                target.to_str().expect("utf-8 target"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_save_preserves_auth_and_data_path_conventions() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        let target = dir.path().join("working.cf");
        let data_path = dir.path().join("ibcmd-data");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = IbcmdConnection::from_infobase(
            &InfobaseConfig::file("File=/ib")
                .with_credentials(Some("admin".to_owned()), Some("secret".to_owned())),
        )
        .expect("connection");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        )
        .with_data_path(data_path.clone());

        dsl.config_save(&target, false, None)
            .expect("save working configuration");

        let args = fs::read_to_string(args_log)
            .expect("args")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "infobase".to_owned(),
                "--data".to_owned(),
                data_path.display().to_string(),
                "--db-path".to_owned(),
                "/ib".to_owned(),
                "config".to_owned(),
                "save".to_owned(),
                "--user".to_owned(),
                "admin".to_owned(),
                "--password".to_owned(),
                "secret".to_owned(),
                target.display().to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_export_full_with_data_path_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        let data_path = dir.path().join("ibcmd-data");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        )
        .with_data_path(data_path.clone());

        dsl.config_export_full(dir.path(), None).expect("export");

        let args = fs::read_to_string(args_log).expect("args");
        let args = args.lines().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "infobase".to_owned(),
                "--data".to_owned(),
                data_path.display().to_string(),
                "--db-path".to_owned(),
                "/ib".to_owned(),
                "config".to_owned(),
                "export".to_owned(),
                "--force".to_owned(),
                dir.path().display().to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_export_full_repeatedly_executes_fresh_script_without_etxtbsy() {
        for _ in 0..32 {
            let dir = tempdir().expect("tempdir");
            let script = dir.path().join("ibcmd");
            let args_log = dir.path().join("args.log");
            write_script(
                &script,
                &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
            );
            let runner = ProcessExecutor;
            let conn = file_connection("File=/ib");
            let dsl = IbcmdDsl::new(
                script,
                conn,
                &runner as &dyn ProcessRunner,
                ProcessExecutionPolicy::default(),
            );

            dsl.config_export_full(dir.path(), None).expect("export");

            let args = fs::read_to_string(args_log).expect("args");
            assert!(args.contains("export"));
            assert!(args.contains("--force"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_export_incremental_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.config_export_incremental(dir.path(), None)
            .expect("export");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("export"));
        assert!(args.contains("--sync"));
    }

    #[cfg(unix)]
    #[test]
    fn run_returns_stdout_stderr_without_platform_log() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        write_script(&script, "echo out; echo err 1>&2; exit 7");
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        let result = dsl
            .config_apply(None, DynamicUpdateMode::Auto)
            .expect("apply");

        assert_eq!(result.process.exit_code, 7);
        assert_eq!(result.process.stdout.trim(), "out");
        assert_eq!(result.process.stderr.trim(), "err");
        assert!(result.platform_log_path.is_none());
        assert!(result.platform_log.is_none());
        assert!(result.platform_log_read_error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn infobase_create_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        let outcome = dsl.ensure_infobase_create().expect("create");

        let args = fs::read_to_string(args_log).expect("args");
        assert_eq!(outcome.status, IbcmdInfobaseCreateStatus::Created);
        assert!(args.contains("infobase"));
        assert!(args.contains("create"));
        assert!(args.contains("infobase\n--db-path\n/ib\ncreate"));
    }

    #[cfg(unix)]
    #[test]
    fn server_infobase_create_adds_create_database_and_asks_the_infobase() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        // The create fails and the infobase still reads, so it was already there. The message
        // the platform prints plays no part.
        write_script(
            &script,
            &format!(
                "printf '%s\\n' \"$@\" >> \"{}\"\nif printf '%s' \"$*\" | grep -F -q -- 'generation-id'; then exit 0; fi\nprintf 'already exists\\n' >&2\nexit 17",
                args_log.display()
            ),
        );
        let runner = ProcessExecutor;
        let conn = IbcmdConnection::from_infobase(&InfobaseConfig::server(
            "Srvr=demo;Ref=test",
            InfobaseDbmsConfig::new("PostgreSQL", "localhost", "demo")
                .with_credentials(Some("postgres".to_owned()), Some("secret".to_owned())),
        ))
        .expect("connection");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        let outcome = dsl.ensure_infobase_create().expect("ensure");

        assert_eq!(outcome.status, IbcmdInfobaseCreateStatus::AlreadyExists);
        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("--create-database"));
        assert!(args.contains("--dbms\nPostgreSQL"));
        assert!(args.contains("--database-server\nlocalhost"));
        assert!(args.contains("--database-name\ndemo"));
        assert!(args.contains("--database-user\npostgres"));
        assert!(args.contains("--database-password\nsecret"));
    }

    /// Two tests used to stand here, proving that the phrase «уже существует» was benign in
    /// upper case and that «ошибка авторизации» next to it was not. Both read the platform's
    /// wording, which DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES forbids, and the fact they protected is now asked of the
    /// infobase instead.
    #[cfg(unix)]
    #[test]
    fn a_failed_create_over_a_readable_infobase_is_already_there() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        // Creation fails; reading the generation id succeeds. No message says so.
        write_script(
            &script,
            &format!(
                "printf '%s\\n' \"$@\" >> \"{}\"\nif [ \"$2\" = \"create\" ] || [ \"$1\" = \"create\" ]; then exit 255; fi\nif printf '%s' \"$*\" | grep -F -q -- 'generation-id'; then exit 0; fi\nexit 255",
                args_log.display()
            ),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        let outcome = dsl.ensure_infobase_create().expect("create outcome");

        assert_eq!(outcome.status, IbcmdInfobaseCreateStatus::AlreadyExists);
        let args = fs::read_to_string(&args_log).expect("args");
        assert!(
            args.contains("generation-id"),
            "the question is asked of the infobase: {args}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_create_over_an_unreadable_infobase_stays_a_failure() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        // Everything fails, including the read — an unwritable path and a wrong user both land
        // here, and neither is silently turned into "already there".
        write_script(&script, "exit 255");
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        let outcome = dsl.ensure_infobase_create().expect("create outcome");

        assert_eq!(outcome.status, IbcmdInfobaseCreateStatus::Failed);
    }

    #[cfg(unix)]
    #[test]
    fn infobase_extension_update_properties_builds_expected_args() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        let args_log = dir.path().join("args.log");
        write_script(
            &script,
            &format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0", args_log.display()),
        );
        let runner = ProcessExecutor;
        let conn = file_connection("File=/ib");
        let dsl = IbcmdDsl::new(
            script,
            conn,
            &runner as &dyn ProcessRunner,
            ProcessExecutionPolicy::default(),
        );

        dsl.infobase_extension_update_properties("client_mcp", false, false)
            .expect("update");

        let args = fs::read_to_string(args_log).expect("args");
        assert!(args.contains("extension"));
        assert!(args.contains("update"));
        assert!(args.contains("infobase\n--db-path\n/ib\nconfig\nextension\nupdate"));
        assert!(args.contains("--name\nclient_mcp"));
        assert!(args.contains("--safe-mode\nno"));
        assert!(args.contains("--unsafe-action-protection\nno"));
    }
}
