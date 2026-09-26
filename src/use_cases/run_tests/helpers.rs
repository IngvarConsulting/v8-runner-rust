use std::time::{Duration, Instant};

use crate::config::model::AppConfig;
use crate::domain::artifact::ArtifactSet;
use crate::domain::execution::{
    ExecutionInterruptionDetails, ExecutionInterruptionPhase, ExecutionOutcome, ExecutionStatus,
    ExecutionStepKind, ExecutionStepStatus, StepResult,
};
use crate::domain::runner::{LaunchClientModeRequest, LaunchOptions, RunnerKind};
use crate::domain::test::{
    test_execution_error, TestErrorKind, TestOutputMode, TestReport, TestRunResult, TestTarget,
};
use crate::platform::enterprise::{EnterpriseDsl, EnterpriseError};
use crate::platform::locator::UtilityType;
use crate::platform::process::{ProcessError, ProcessInterruptionReason};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::path::is_safe_path_segment;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::{
    interruption_record, process_interruption_details, SafePoint, SafePointCancel,
};
use crate::use_cases::launch_keys::vanessa_enterprise_launch_keys;
use crate::use_cases::request::{TestRequest as TestArgs, TestScopeRequest as TestScope};
use crate::use_cases::result::UseCaseError;

use super::{build_yaxunit_config, prepare_vanessa_run, PreparedRun, RunArtifacts};

pub(super) fn make_test_result(
    target: TestTarget,
    mode: TestOutputMode,
    outcome: ExecutionOutcome<TestReport>,
    warnings: Vec<String>,
    steps: Vec<StepResult>,
    duration_ms: u64,
) -> TestRunResult {
    TestRunResult::from_outcome(outcome, target, mode, warnings, steps, duration_ms)
}

pub(super) fn succeeded_step(
    name: &str,
    kind: ExecutionStepKind,
    duration_ms: u64,
    message: impl Into<String>,
) -> StepResult {
    StepResult::succeeded(name, kind, duration_ms).with_message(message)
}

pub(super) fn skipped_step(
    name: &str,
    kind: ExecutionStepKind,
    duration_ms: u64,
    message: impl Into<String>,
) -> StepResult {
    StepResult::new(name, kind, ExecutionStepStatus::Skipped, duration_ms).with_message(message)
}

pub(super) fn failed_step(
    name: &str,
    kind: ExecutionStepKind,
    duration_ms: u64,
    message: impl Into<String>,
) -> StepResult {
    let message = message.into();
    StepResult::failed(name, kind, duration_ms)
        .with_message(message.clone())
        .with_diagnostics(vec![message])
}

pub(super) fn degraded_step(
    name: &str,
    kind: ExecutionStepKind,
    duration_ms: u64,
    message: impl Into<String>,
) -> StepResult {
    let message = message.into();
    StepResult::degraded(name, kind, duration_ms)
        .with_message(message.clone())
        .with_diagnostics(vec![message])
}

pub(super) fn with_retained_artifacts(
    mut outcome: ExecutionOutcome<TestReport>,
    retained_paths: Option<ArtifactSet>,
) -> ExecutionOutcome<TestReport> {
    if let Some(retained_paths) = retained_paths {
        outcome = outcome.with_artifacts(retained_paths);
    }
    outcome
}

pub(super) fn interrupted_test_failure(
    context: &ExecutionContext,
    target: &TestTarget,
    mode: &TestOutputMode,
    warnings: &[String],
    steps: &[StepResult],
    started: Instant,
) -> Option<super::TestExecutionFailure> {
    let cancel = SafePointCancel::noticed(context, SafePoint::Command)?;
    let outcome = ExecutionOutcome::new(cancel.status())
        .with_diagnostics(vec![cancel.message().to_owned()])
        .with_interruptions(vec![cancel.record()]);
    let result = make_test_result(
        target.clone(),
        mode.clone(),
        outcome,
        warnings.to_vec(),
        steps.to_vec(),
        started.elapsed().as_millis() as u64,
    );
    Some(super::TestExecutionFailure::with_payload(
        cancel.into_error(),
        result,
    ))
}

/// Итог теста, чью сборку-предпосылку остановил отказ `error`. Сборку прервала отмена — тест
/// отвечает прерыванием там, где она её застала, а не отказом сборки; прочий отказ —
/// `build_failed`.
pub(super) fn build_prerequisite_failure(
    error: &UseCaseError,
    step: StepResult,
    summary: &str,
) -> (StepResult, ExecutionOutcome<TestReport>) {
    match error.cancellation() {
        Some(at) => (
            step,
            ExecutionOutcome::new(ExecutionStatus::Cancelled)
                .with_diagnostics(vec![summary.to_owned()])
                .with_interruptions(vec![interruption_record(
                    at,
                    ExecutionInterruptionPhase::ProviderCommand,
                    summary,
                )]),
        ),
        None => (
            step.with_errors(vec![test_execution_error(
                TestErrorKind::BuildFailed,
                summary,
            )]),
            ExecutionOutcome::new(ExecutionStatus::Failed)
                .with_diagnostics(vec![summary.to_owned()])
                .with_errors(vec![test_execution_error(
                    TestErrorKind::BuildFailed,
                    summary,
                )]),
        ),
    }
}

pub(super) fn validate_runner_profile_id(profile_id: &str) -> Result<&str, AppError> {
    if !is_safe_path_segment(profile_id) {
        return Err(AppError::Validation(format!(
            "runner profile contains unsafe path characters: {profile_id}"
        )));
    }
    Ok(profile_id)
}

pub(super) fn build_summary(result: &crate::domain::build::BuildResult) -> String {
    if result.ok {
        "build completed".to_owned()
    } else {
        result
            .steps
            .iter()
            .find(|step| !step.ok)
            .map(|step| {
                format!(
                    "build failed at source-set '{}' ({})",
                    step.source_set,
                    step.message.as_deref().unwrap_or("unknown error")
                )
            })
            .unwrap_or_else(|| "build failed".to_owned())
    }
}

pub(super) fn prepared_run_summary(prepared_run: &PreparedRun) -> String {
    match prepared_run {
        PreparedRun::YaXUnit => "YaXUnit config written".to_owned(),
        PreparedRun::Vanessa { .. } => "Vanessa Automation params written".to_owned(),
    }
}

pub(super) fn validate_target(
    runner_kind: &RunnerKind,
    scope: &TestScope,
) -> Result<TestTarget, AppError> {
    match scope {
        TestScope::All => Ok(TestTarget::All),
        TestScope::Module { name } => {
            if *runner_kind == RunnerKind::Vanessa {
                return Err(AppError::Validation(
                    "Vanessa Automation supports only 'test va' without module scope".to_owned(),
                ));
            }
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                return Err(AppError::Validation(
                    "test module requires a non-empty module name".to_owned(),
                ));
            }
            Ok(TestTarget::Module {
                name: trimmed.to_owned(),
            })
        }
    }
}

pub(super) fn prepare_runner_artifacts(
    config: &AppConfig,
    args: &TestArgs,
    target: &TestTarget,
    artifacts: &mut RunArtifacts,
) -> Result<PreparedRun, AppError> {
    match args.execution.profile.kind {
        RunnerKind::YaXUnit => {
            tracing::debug!(
                path = %artifacts.config_json.display(),
                "writing YaXUnit configuration"
            );
            let config_payload = build_yaxunit_config(target, artifacts);
            super::write_json_file(&artifacts.config_json, &config_payload).map_err(|error| {
                AppError::Runtime(format!("failed to write YaXUnit config: {error}"))
            })?;
            Ok(PreparedRun::YaXUnit)
        }
        RunnerKind::Vanessa => prepare_vanessa_run(config, args, artifacts),
        ref other => Err(AppError::Validation(format!(
            "unsupported test runner kind: {other:?}"
        ))),
    }
}

pub(super) fn build_enterprise_dsl<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    artifacts: &'a RunArtifacts,
    prepared_run: &PreparedRun,
    launch: &LaunchOptions,
    runner: &'a dyn crate::platform::process::ProcessRunner,
    client_mode: LaunchClientModeRequest,
    timeout_override_ms: Option<u64>,
) -> Result<EnterpriseDsl<'a>, AppError> {
    let mut utilities = PlatformUtilities::from_config(config);
    let utility = match client_mode {
        LaunchClientModeRequest::Designer => UtilityType::V8,
        LaunchClientModeRequest::Thin => UtilityType::V8C,
        LaunchClientModeRequest::Thick | LaunchClientModeRequest::Ordinary => UtilityType::V8,
    };
    let location = utilities.locate(utility).map_err(AppError::from)?;
    tracing::debug!(
        additional_launch_keys = ?config.tools.enterprise.additional_launch_keys,
        "resolved enterprise additional launch keys"
    );
    let additional_launch_keys = effective_enterprise_launch_keys(config, prepared_run, launch);
    Ok(EnterpriseDsl::new(
        location.path,
        config.v8_connection(),
        additional_launch_keys,
        client_mode.into(),
        runner,
        artifacts.platform_log.clone(),
        context.process_policy(
            InterruptionSafetyClass::GracefulThenKill,
            Some(
                timeout_override_ms
                    .map(Duration::from_millis)
                    .unwrap_or_else(|| Duration::from_secs(config.tests.execution_timeout_seconds)),
            ),
        ),
    ))
}

fn effective_enterprise_launch_keys(
    config: &AppConfig,
    prepared_run: &PreparedRun,
    launch: &LaunchOptions,
) -> Vec<String> {
    if matches!(prepared_run, PreparedRun::Vanessa { .. }) {
        return vanessa_enterprise_launch_keys(
            &config.tools.enterprise.additional_launch_keys,
            launch,
        );
    }
    config.tools.enterprise.additional_launch_keys.clone()
}

pub(super) fn build_platform_launch(
    base: &LaunchOptions,
    prepared_run: &PreparedRun,
    artifacts: &RunArtifacts,
) -> LaunchOptions {
    let mut launch = base.clone();
    match prepared_run {
        PreparedRun::YaXUnit => {
            launch.c = Some(format!(
                "RunUnitTests={}",
                crate::platform::enterprise::normalize_launch_payload_path(&artifacts.config_json)
            ));
            launch.execute = None;
        }
        PreparedRun::Vanessa {
            epf_path,
            params_path,
        } => {
            launch = crate::use_cases::vanessa::apply_test_player_launch(
                base,
                &crate::use_cases::vanessa::VanessaLaunch {
                    epf_path: epf_path.clone(),
                    params_path: params_path.clone(),
                },
            );
        }
    }
    launch
}

pub(super) fn collect_diagnostics(
    platform_result: &crate::platform::result::PlatformCommandResult,
    mut diagnostics: Vec<String>,
    config: &AppConfig,
) -> Vec<String> {
    if !platform_result.process.stderr.trim().is_empty() {
        diagnostics.push(super::sanitize_text(
            &platform_result.process.stderr,
            config,
        ));
    }
    if let Some(log) = &platform_result.platform_log {
        let trimmed = log.trim();
        if !trimmed.is_empty() {
            diagnostics.push(super::limit_excerpt(&super::sanitize_text(trimmed, config)));
        }
    }
    diagnostics
}

pub(super) fn enterprise_error_kind(
    error: EnterpriseError,
) -> (
    Option<TestErrorKind>,
    AppError,
    Option<ExecutionInterruptionDetails>,
    ExecutionStatus,
) {
    let error = AppError::from(error);
    // Отмену и её место называет ошибка: снятый прогон — фаза `run`, отказ запустить его по
    // отмене — безопасная точка.
    if let Some(at) = error.cancellation() {
        let message = "enterprise test run cancelled";
        return (
            None,
            error.with_context(message),
            Some(interruption_record(
                at,
                ExecutionInterruptionPhase::Run,
                message,
            )),
            ExecutionStatus::Cancelled,
        );
    }
    let kind = match &error {
        AppError::PlatformProcess(ProcessError::TimedOut { .. }) => {
            return (
                None,
                AppError::Runtime("enterprise test run timed out".to_owned()),
                Some(process_interruption_details(
                    ProcessInterruptionReason::TimedOut,
                    ExecutionInterruptionPhase::Run,
                    false,
                    "enterprise test run timed out",
                )),
                ExecutionStatus::TimedOut,
            );
        }
        AppError::PlatformProcess(ProcessError::StartupCheckFailed { .. }) => {
            TestErrorKind::EnterpriseStartupCheckFailed
        }
        AppError::PlatformProcess(ProcessError::ExitedEarly { .. }) => {
            TestErrorKind::EnterpriseExitedEarly
        }
        AppError::PlatformProcess(ProcessError::StdoutLogIo { .. }) => {
            TestErrorKind::EnterpriseStdoutLogIo
        }
        AppError::PlatformProcess(ProcessError::StderrLogIo { .. }) => {
            TestErrorKind::EnterpriseStderrLogIo
        }
        AppError::PlatformProcess(
            ProcessError::SpawnFailed { .. } | ProcessError::ManagedSpawnUnsupported { .. },
        ) => TestErrorKind::EnterpriseSpawnFailed,
        // Сюда не доходит ничто: отмену разобрали выше, а ошибка прогона — всегда ошибка
        // процесса. Новый вид ошибки процесса должен получить здесь свою строку.
        other => {
            debug_assert!(
                false,
                "an enterprise run failure is not classified: {other}"
            );
            TestErrorKind::EnterpriseSpawnFailed
        }
    };
    (Some(kind), error, None, ExecutionStatus::Failed)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::enterprise_error_kind;
    use crate::domain::execution::{
        ExecutionInterruptionKind, ExecutionInterruptionPhase, ExecutionStatus,
    };
    use crate::domain::test::TestErrorKind;
    use crate::platform::enterprise::EnterpriseError;
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;

    fn assert_process_mapping(
        process_error: ProcessError,
        expected_kind: TestErrorKind,
        assert_typed_error: impl FnOnce(AppError),
    ) {
        let (kind, app_error, interruption, status) =
            enterprise_error_kind(EnterpriseError::Spawn(process_error));

        assert_eq!(kind, Some(expected_kind));
        assert_typed_error(app_error);
        assert!(interruption.is_none());
        assert_eq!(status, ExecutionStatus::Failed);
    }

    #[test]
    fn enterprise_process_errors_keep_distinct_test_error_kinds() {
        assert_process_mapping(
            ProcessError::SpawnFailed {
                cmd: "1cv8c ENTERPRISE".to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
            },
            TestErrorKind::EnterpriseSpawnFailed,
            |error| {
                assert!(matches!(
                    error,
                    AppError::PlatformProcess(ProcessError::SpawnFailed { .. })
                ));
            },
        );
        assert_process_mapping(
            ProcessError::StartupCheckFailed {
                cmd: "1cv8c ENTERPRISE".to_owned(),
                source: std::io::Error::other("probe failed"),
            },
            TestErrorKind::EnterpriseStartupCheckFailed,
            |error| {
                assert!(matches!(
                    error,
                    AppError::PlatformProcess(ProcessError::StartupCheckFailed { .. })
                ));
            },
        );
        assert_process_mapping(
            ProcessError::ExitedEarly {
                cmd: "1cv8c ENTERPRISE".to_owned(),
                exit_code: 17,
            },
            TestErrorKind::EnterpriseExitedEarly,
            |error| {
                assert!(matches!(
                    error,
                    AppError::PlatformProcess(ProcessError::ExitedEarly { .. })
                ));
            },
        );
        assert_process_mapping(
            ProcessError::StdoutLogIo {
                path: PathBuf::from("stdout.log"),
                source: std::io::Error::other("stdout write"),
            },
            TestErrorKind::EnterpriseStdoutLogIo,
            |error| {
                assert!(matches!(
                    error,
                    AppError::PlatformProcess(ProcessError::StdoutLogIo { .. })
                ));
            },
        );
        assert_process_mapping(
            ProcessError::StderrLogIo {
                path: PathBuf::from("stderr.log"),
                source: std::io::Error::other("stderr write"),
            },
            TestErrorKind::EnterpriseStderrLogIo,
            |error| {
                assert!(matches!(
                    error,
                    AppError::PlatformProcess(ProcessError::StderrLogIo { .. })
                ));
            },
        );
    }

    /// Снятый прогон — прерывание в фазе `run`, отказ запустить его по отмене — безопасная
    /// точка; род отказа у обоих — отмена.
    #[test]
    fn a_cancelled_run_is_classified_by_where_it_stopped() {
        for (delivered, phase) in [
            (true, ExecutionInterruptionPhase::Run),
            (false, ExecutionInterruptionPhase::CommandBoundary),
        ] {
            let (kind, app_error, interruption, status) =
                enterprise_error_kind(EnterpriseError::Spawn(ProcessError::Cancelled {
                    cmd: "1cv8c ENTERPRISE".to_owned(),
                    delivered,
                }));

            assert_eq!(kind, None);
            assert_eq!(
                app_error.cancellation(),
                Some(crate::support::error::CancelledAt::after(delivered))
            );
            let interruption = interruption.expect("a cancelled run is an interruption");
            assert_eq!(interruption.kind, ExecutionInterruptionKind::Cancelled);
            assert!(!interruption.deferred);
            assert_eq!(interruption.phase, Some(phase), "delivered: {delivered}");
            assert_eq!(status, ExecutionStatus::Cancelled);
        }
    }

    /// Сборка-предпосылка, остановленная отменой, — прерывание теста там, где отмена её
    /// застала: безопасная точка сборки — `command_boundary`, снятый исполнитель сборки —
    /// `provider_command`. Прочий отказ сборки остаётся `build_failed` (#308).
    #[test]
    fn a_build_prerequisite_stopped_by_a_cancellation_is_an_interruption() {
        use crate::domain::execution::{ExecutionStepKind, StepResult};
        use crate::support::error::CancelledAt;
        use crate::use_cases::result::UseCaseError;

        for (at, phase) in [
            (
                CancelledAt::Boundary,
                ExecutionInterruptionPhase::CommandBoundary,
            ),
            (
                CancelledAt::Work,
                ExecutionInterruptionPhase::ProviderCommand,
            ),
        ] {
            let error = UseCaseError::from(AppError::Cancelled {
                message: "build cancelled".to_owned(),
                at,
            });
            let (step, outcome) = super::build_prerequisite_failure(
                &error,
                StepResult::failed("build", ExecutionStepKind::PlatformCommand, 0),
                "build cancelled",
            );

            assert!(step.errors.is_empty(), "{at:?}: {step:?}");
            assert_eq!(outcome.status, ExecutionStatus::Cancelled);
            assert!(outcome.errors.is_empty(), "{at:?}: {:?}", outcome.errors);
            let [interruption] = outcome.interruptions.as_slice() else {
                panic!("one interruption expected: {:?}", outcome.interruptions);
            };
            assert_eq!(interruption.kind, ExecutionInterruptionKind::Cancelled);
            assert_eq!(interruption.phase, Some(phase), "{at:?}");
        }

        let failed = UseCaseError::from(AppError::Platform("load failed".to_owned()));
        let (step, outcome) = super::build_prerequisite_failure(
            &failed,
            StepResult::failed("build", ExecutionStepKind::PlatformCommand, 0),
            "load failed",
        );
        assert_eq!(step.errors[0].code, TestErrorKind::BuildFailed.code());
        assert_eq!(outcome.status, ExecutionStatus::Failed);
        assert!(outcome.interruptions.is_empty());
    }

    #[test]
    fn enterprise_timeout_keeps_interruption_contract() {
        let (kind, app_error, interruption, status) =
            enterprise_error_kind(EnterpriseError::Spawn(ProcessError::TimedOut {
                cmd: "1cv8c ENTERPRISE".to_owned(),
                timeout_ms: 500,
            }));

        assert_eq!(kind, None);
        assert!(matches!(app_error, AppError::Runtime(_)));
        let interruption = interruption.expect("a timed-out run is an interruption");
        assert_eq!(interruption.kind, ExecutionInterruptionKind::TimedOut);
        assert!(!interruption.deferred);
        assert_eq!(interruption.phase, Some(ExecutionInterruptionPhase::Run));
        assert_eq!(status, ExecutionStatus::TimedOut);
    }
}
