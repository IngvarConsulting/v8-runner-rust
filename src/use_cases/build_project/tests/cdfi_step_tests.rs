use super::*;
use crate::domain::build::CdfiRecoveryAction;
use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessInterruption, ProcessInterruptionAction,
    ProcessInterruptionReason, ProcessInterruptionSafety, ProcessRequest, ProcessResult,
    ProcessRunner, SpawnResult,
};
use crate::use_cases::build_project::{execute_source_set_step, StepCommit};
use crate::use_cases::context::ExecutionInterruption;
use std::cell::Cell;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

enum Scenario {
    CancelLoad,
    TimeoutLoad(Instant),
    CancelUpdate,
    HashFailure,
    CancelLoadAndBlockRestore,
}

struct MutatingDesigner {
    cdfi: PathBuf,
    storage: PathBuf,
    scenario: Scenario,
    calls: Cell<usize>,
}

impl ProcessRunner for MutatingDesigner {
    fn run(&self, _: &ProcessRequest) -> Result<ProcessResult, ProcessError> {
        panic!("Designer must dispatch with an explicit interruption policy")
    }

    fn run_with_timeout(
        &self,
        _: &ProcessRequest,
        _: Duration,
    ) -> Result<ProcessResult, ProcessError> {
        panic!("Designer must preserve the full policy")
    }

    fn spawn(&self, _: &ProcessRequest) -> Result<SpawnResult, ProcessError> {
        panic!("build must wait for Designer")
    }

    fn run_with_policy(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError> {
        assert_eq!(
            policy.safety,
            ProcessInterruptionSafety::CriticalNonAbortable
        );
        self.calls.set(self.calls.get() + 1);
        let is_load = request.args.iter().any(|arg| arg == "/LoadConfigFromFiles");
        let mut interruption = None;
        if is_load {
            fs::write(&self.cdfi, b"platform generation").expect("mutate version file");
            match self.scenario {
                Scenario::CancelLoad | Scenario::CancelLoadAndBlockRestore => {
                    policy.cancellation.cancel();
                    if matches!(self.scenario, Scenario::CancelLoadAndBlockRestore) {
                        fs::remove_file(&self.cdfi).expect("replace version file");
                        fs::create_dir(&self.cdfi).expect("obstruct restoration");
                    }
                }
                Scenario::TimeoutLoad(deadline) => {
                    // Observe the configured deadline, rather than assuming a sleep raced it.
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                Scenario::CancelUpdate | Scenario::HashFailure => {}
            }
        } else {
            assert!(request.args.iter().any(|arg| arg == "/UpdateDBCfg"));
            match self.scenario {
                Scenario::CancelUpdate => {
                    policy.cancellation.cancel();
                    interruption = Some(ProcessInterruption {
                        reason: ProcessInterruptionReason::Cancelled,
                        action: ProcessInterruptionAction::Deferred,
                    });
                }
                Scenario::HashFailure => {
                    fs::create_dir_all(self.storage.join("obstruction"))
                        .expect("obstruct hash commit after update success");
                }
                Scenario::CancelLoad
                | Scenario::TimeoutLoad(_)
                | Scenario::CancelLoadAndBlockRestore => panic!("must stop before update"),
            }
        }
        Ok(ProcessResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            interruption,
        })
    }
}

fn exercise(scenario: Scenario) {
    let dir = tempdir().expect("tempdir");
    let base = dir.path().join("source");
    let work = dir.path().join("work");
    create_source_tree(&base);
    let config = build_config(
        &base,
        &work,
        &dir.path().join("unused-platform"),
        20,
        SourceFormat::Designer,
        BuilderBackend::Designer,
    );
    let source_set = &config.source_sets[0];
    let source = SourceSetContext::new(
        source_set.name.clone(),
        base.join(&source_set.path),
        "cdfi-step-test".to_owned(),
    );
    let cdfi = source.path().join("ConfigDumpInfo.xml");
    let original = b"\xef\xbb\xbf<ConfigDumpInfo/>\r\n";
    fs::write(&cdfi, original).expect("baseline");
    let context =
        ExecutionContext::cli(CommandName::Build).with_cancellation(CancellationToken::new());
    // Give capture ample time; the fake waits for this exact deadline after mutating CDFI.
    let (context, scenario) = match scenario {
        Scenario::TimeoutLoad(_) => {
            let deadline = Instant::now() + Duration::from_secs(1);
            (
                context.with_deadline(Some(deadline)),
                Scenario::TimeoutLoad(deadline),
            )
        }
        scenario => (context, scenario),
    };
    let runner = MutatingDesigner {
        cdfi: cdfi.clone(),
        storage: source.storage_path(&work),
        scenario,
        calls: Cell::new(0),
    };
    let outcome = execute_source_set_step(
        &context,
        &config,
        Path::new("fake-designer"),
        &runner,
        source_set,
        &source,
        &source,
        0,
        None,
        &StepCommit::RescanFull {
            recover_storage: false,
        },
    );
    match runner.scenario {
        Scenario::CancelLoad | Scenario::TimeoutLoad(_) => {
            let failure = outcome.expect_err("interruption before update");
            let expected = match runner.scenario {
                Scenario::TimeoutLoad(_) => ExecutionInterruption::TimedOut,
                _ => ExecutionInterruption::Cancelled,
            };
            // Preserve the existing build boundary classification; recovery must not remap it.
            assert_eq!(failure.error.kind(), UseCaseErrorKind::Runtime);
            assert_eq!(context.interruption(), Some(expected));
            assert_eq!(runner.calls.get(), 1);
            assert_eq!(fs::read(&cdfi).expect("restored"), original);
            assert_eq!(
                failure
                    .payload
                    .expect("step data")
                    .cdfi_recovery
                    .expect("receipt")
                    .action,
                CdfiRecoveryAction::Restored
            );
            assert!(!runner.storage.exists());
        }
        Scenario::CancelUpdate => {
            let result = outcome.expect("successful critical update defers cancellation");
            assert_eq!(runner.calls.get(), 2);
            assert_eq!(fs::read(&cdfi).expect("committed"), b"platform generation");
            assert_eq!(
                result.cdfi_recovery.expect("receipt").action,
                CdfiRecoveryAction::NotNeeded
            );
            assert!(!result.warnings.is_empty());
            assert!(runner.storage.is_file());
        }
        Scenario::HashFailure => {
            let failure = outcome.expect_err("hash persistence fails after applied update");
            assert_eq!(runner.calls.get(), 2);
            assert_eq!(fs::read(&cdfi).expect("committed"), b"platform generation");
            assert_eq!(
                failure
                    .payload
                    .expect("step data")
                    .cdfi_recovery
                    .expect("receipt")
                    .action,
                CdfiRecoveryAction::NotNeeded
            );
        }
        Scenario::CancelLoadAndBlockRestore => {
            let failure = outcome.expect_err("cancelled with failed rollback");
            assert_eq!(failure.error.kind(), UseCaseErrorKind::Runtime);
            assert_eq!(
                context.interruption(),
                Some(ExecutionInterruption::Cancelled)
            );
            let summary = failure
                .payload
                .expect("step data")
                .cdfi_recovery
                .expect("receipt");
            assert_eq!(summary.action, CdfiRecoveryAction::Failed);
            assert_eq!(
                fs::read(summary.snapshot_path.expect("retained snapshot")).expect("backup"),
                original
            );
            assert!(cdfi.is_dir());
        }
    }
}

#[test]
fn cancellation_before_update_restores_cdfi_even_with_pending_interruption() {
    exercise(Scenario::CancelLoad);
}

#[test]
fn timeout_before_update_restores_cdfi() {
    exercise(Scenario::TimeoutLoad(Instant::now()));
}

#[test]
fn successful_update_defers_cancel_and_keeps_new_cdfi() {
    exercise(Scenario::CancelUpdate);
}

#[test]
fn successful_update_then_hash_failure_never_restores_old_cdfi() {
    exercise(Scenario::HashFailure);
}

#[test]
fn failed_restore_keeps_original_cancellation_kind_and_backup() {
    exercise(Scenario::CancelLoadAndBlockRestore);
}
