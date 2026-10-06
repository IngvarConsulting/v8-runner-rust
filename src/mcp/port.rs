use std::sync::Arc;

use crate::config::model::AppConfig;
use crate::domain::build::BuildResult;
use crate::domain::dump::DumpResult;
use crate::domain::launch::LaunchResult;
use crate::domain::syntax::SyntaxCheckResult;
use crate::domain::test::TestRunResult;
use crate::use_cases::build_project;
use crate::use_cases::check_syntax;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::dump_config;
use crate::use_cases::infobase_lock::BaseAccess;
use crate::use_cases::launch_app;
use crate::use_cases::request::{
    BuildRequest, DumpRequest, LaunchRequest, SyntaxRequest, TestRequest,
};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use crate::use_cases::run_tests;
use crate::use_cases::transport::dispatch_with_workspace_lock;
use tracing::warn;

/// Thin indirection layer used by the MCP service to call use cases.
pub trait McpUseCasePort {
    fn build_project(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &BuildRequest,
    ) -> UseCaseResult<BuildResult>;

    fn run_tests(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &TestRequest,
    ) -> UseCaseResult<TestRunResult>;

    fn dump_config(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &DumpRequest,
    ) -> UseCaseResult<DumpResult>;

    fn launch_app(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &LaunchRequest,
    ) -> UseCaseResult<LaunchResult>;

    fn check_syntax(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &SyntaxRequest,
    ) -> UseCaseResult<SyntaxCheckResult>;
}

/// Production port implementation delegating directly to use cases.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultMcpUseCasePort;

impl McpUseCasePort for DefaultMcpUseCasePort {
    fn build_project(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &BuildRequest,
    ) -> UseCaseResult<BuildResult> {
        with_workspace_lock(context, config, BaseAccess::Writes, || {
            build_project::execute(context, config, request)
        })
    }

    fn run_tests(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &TestRequest,
    ) -> UseCaseResult<TestRunResult> {
        with_workspace_lock(context, config, BaseAccess::Writes, || {
            run_tests::execute(context, config, request)
        })
    }

    fn dump_config(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &DumpRequest,
    ) -> UseCaseResult<DumpResult> {
        with_workspace_lock(context, config, BaseAccess::Writes, || {
            dump_config::execute(context, config, request)
        })
    }

    fn launch_app(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &LaunchRequest,
    ) -> UseCaseResult<LaunchResult> {
        with_workspace_lock(context, config, BaseAccess::Writes, || {
            launch_app::execute(context, config, request)
        })
    }

    fn check_syntax(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &SyntaxRequest,
    ) -> UseCaseResult<SyntaxCheckResult> {
        with_workspace_lock(context, config, request.base_access(), || {
            check_syntax::execute(context, config, request)
        })
    }
}

/// Граница порта: замок `workPath`, затем замок базы. Инструменты MCP — команды записи
/// или базу не открывают, поэтому предупреждения команды чтения здесь не бывает; если оно
/// всё же придёт, оно уходит в журнал.
fn with_workspace_lock<T>(
    context: &ExecutionContext,
    config: &AppConfig,
    base: BaseAccess,
    run: impl FnOnce() -> UseCaseResult<T>,
) -> UseCaseResult<T> {
    let command = context.command();
    let before_dispatch = |warning: Option<&str>| {
        if let Some(warning) = warning {
            warn!(command = command.as_str(), "{warning}");
        }
        Ok(())
    };
    match dispatch_with_workspace_lock(config, command, base, before_dispatch, run) {
        Ok(result) => result,
        Err(refusal) => Err(UseCaseFailure::without_payload(refusal.error)),
    }
}

impl<T> McpUseCasePort for Arc<T>
where
    T: McpUseCasePort + ?Sized,
{
    fn build_project(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &BuildRequest,
    ) -> UseCaseResult<BuildResult> {
        (**self).build_project(context, config, request)
    }

    fn run_tests(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &TestRequest,
    ) -> UseCaseResult<TestRunResult> {
        (**self).run_tests(context, config, request)
    }

    fn dump_config(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &DumpRequest,
    ) -> UseCaseResult<DumpResult> {
        (**self).dump_config(context, config, request)
    }

    fn launch_app(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &LaunchRequest,
    ) -> UseCaseResult<LaunchResult> {
        (**self).launch_app(context, config, request)
    }

    fn check_syntax(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        request: &SyntaxRequest,
    ) -> UseCaseResult<SyntaxCheckResult> {
        (**self).check_syntax(context, config, request)
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultMcpUseCasePort, McpUseCasePort};
    use crate::config::model::{
        AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::mcp::error::{McpBusinessError, McpBusinessErrorKind, McpErrorCode};
    use crate::support::fs::acquire_advisory_lock;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::BuildRequest;
    use crate::use_cases::result::UseCaseErrorKind;
    use crate::use_cases::workspace_lock::workspace_lock_path;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn sample_config(work_path: &Path) -> AppConfig {
        AppConfig {
            base_path: work_path.join("base"),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file(format!(
                "File={}",
                work_path.with_file_name("ib").display()
            )),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("main"),
            }],
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn default_port_reports_workspace_lock_conflict_before_use_case_dispatch() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");

        let failure = DefaultMcpUseCasePort
            .build_project(
                &ExecutionContext::mcp_stdio(CommandName::Build),
                &config,
                &BuildRequest {
                    dry_run: false,
                    full_rebuild: true,
                    source_set: None,
                },
            )
            .expect_err("busy workspace");

        // Граница замка отказывает своим родом; словарь MCP узок и отвечает
        // `runtime_failure` — решение владельца в #291.
        assert_eq!(failure.error.kind(), UseCaseErrorKind::WorkspaceBusy);
        let mcp_error = McpBusinessError::from_use_case(&failure.error);
        assert_eq!(mcp_error.code, McpErrorCode::RuntimeFailure);
        assert_eq!(mcp_error.kind, McpBusinessErrorKind::Runtime);
        assert!(failure.error.to_string().contains("workspace"));
        assert!(failure.error.to_string().contains("already"));
    }
}
