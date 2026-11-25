//! Versioned process adapter for caller-controlled full-pull publication.
use std::io::{BufRead, Read, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::args::{Cli, DumpArgs};
use super::output::failure_envelope;
use crate::command_envelope::Envelope;
use crate::config::model::SourceFormat;
use crate::domain::capability::ProviderReceipt;
use crate::domain::warning::Warning;
use crate::support::error::AppError;
use crate::support::path::{response_json, response_text};
use crate::use_cases::context::{CommandName, ExecutionContext};
use crate::use_cases::full_pull_publication::{
    ExecutorKind, ExternalFullPull, FullPullControl, FullPullExecutor, FullPullReady, MemoryCommit,
};
use crate::use_cases::infobase_lock::BaseAccess;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};
use crate::use_cases::transport::dispatch_with_workspace_boundary;

const VERSION: u32 = 1;
const MAX_DECISION_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Publication {
    NotPublished,
    Published,
    Unknown,
}
#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Memory {
    Unchanged,
    Saved,
    NotSaved,
}
#[derive(Debug)]
struct State {
    publication: Publication,
    memory: Memory,
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Selected,
    Ready,
}
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Continue,
    Reject,
    Published,
    Rejected,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SelectionDecision {
    Continue,
    Reject,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PublicationDecision {
    Published,
    Rejected,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Selected {
        #[schemars(range(min = 1, max = 1))]
        version: u32,
        #[schemars(regex(
            pattern = r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        ))]
        nonce: String,
        decision: SelectionDecision,
    },
    Ready {
        #[schemars(range(min = 1, max = 1))]
        version: u32,
        #[schemars(regex(
            pattern = r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        ))]
        nonce: String,
        decision: PublicationDecision,
    },
}

impl Reply {
    fn into_parts(self) -> (u32, String, Phase, Decision) {
        match self {
            Self::Selected {
                version,
                nonce,
                decision,
            } => (
                version,
                nonce,
                Phase::Selected,
                match decision {
                    SelectionDecision::Continue => Decision::Continue,
                    SelectionDecision::Reject => Decision::Reject,
                },
            ),
            Self::Ready {
                version,
                nonce,
                decision,
            } => (
                version,
                nonce,
                Phase::Ready,
                match decision {
                    PublicationDecision::Published => Decision::Published,
                    PublicationDecision::Rejected => Decision::Rejected,
                },
            ),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WireExecutorKind {
    Local,
    Attached,
    Remote,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Executor<'a> {
    kind: WireExecutorKind,
    executable: Option<&'a std::path::Path>,
    endpoint: Option<&'a str>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Snapshot {
    Prepared,
    Unavailable,
}

/// One closed output type owns both serialization and the generated wire schema.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum Frame<'a, T: Serialize> {
    Selected {
        #[schemars(range(min = 1, max = 1))]
        version: u32,
        #[schemars(regex(
            pattern = r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        ))]
        nonce: &'a str,
        provider: &'a ProviderReceipt,
        executor: Executor<'a>,
    },
    Ready {
        #[schemars(range(min = 1, max = 1))]
        version: u32,
        #[schemars(regex(
            pattern = r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        ))]
        nonce: &'a str,
        command: &'a str,
        requested_mode: crate::domain::dump::DumpMode,
        mode: crate::domain::dump::ReportedDumpMode,
        extension: Option<&'a str>,
        mode_reason: Option<&'a crate::domain::dump::DumpModeReason>,
        provider_dispatched: bool,
        source_set: &'a str,
        target_path: &'a std::path::Path,
        stage_path: &'a std::path::Path,
        snapshot: Snapshot,
        snapshot_warning: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        merge_baseline: Option<&'a crate::use_cases::merge_baseline::MergeBaselineReceipt>,
    },
    Terminal {
        #[schemars(range(min = 1, max = 1))]
        version: u32,
        #[schemars(regex(
            pattern = r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
        ))]
        nonce: &'a str,
        publication: Publication,
        memory: Memory,
        result: &'a Envelope<T>,
    },
}

#[derive(Debug)]
struct StdioControl {
    nonce: String,
    context: ExecutionContext,
    snapshot_only: bool,
    prepare_merge_baseline: bool,
    input: Mutex<mpsc::Receiver<Result<Vec<u8>, String>>>,
    state: Mutex<State>,
}

impl StdioControl {
    fn new(context: ExecutionContext, snapshot_only: bool, prepare_merge_baseline: bool) -> Self {
        let (send, input) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut input = std::io::stdin().lock();
            loop {
                let mut bytes = Vec::new();
                let result = (&mut input)
                    .take(MAX_DECISION_BYTES + 1)
                    .read_until(b'\n', &mut bytes);
                let item = match result {
                    Ok(0) => Err("publication control input closed".into()),
                    Ok(_) if bytes.len() as u64 > MAX_DECISION_BYTES => {
                        Err("publication control frame exceeds 4096 bytes".into())
                    }
                    Ok(_) if !bytes.ends_with(b"\n") => {
                        Err("publication control frame is incomplete".into())
                    }
                    Ok(_) => Ok(bytes),
                    Err(error) => Err(format!("cannot read publication control: {error}")),
                };
                let terminal = item.is_err();
                if send.send(item).is_err() || terminal {
                    break;
                }
            }
        });
        Self {
            nonce: uuid::Uuid::new_v4().to_string(),
            context,
            snapshot_only,
            prepare_merge_baseline,
            input: Mutex::new(input),
            state: Mutex::new(State {
                publication: Publication::NotPublished,
                memory: Memory::Unchanged,
            }),
        }
    }

    fn emit<T: Serialize>(&self, frame: Frame<'_, T>) -> Result<(), AppError> {
        let encoded = serde_json::to_string(&frame).map_err(|error| {
            AppError::Runtime(format!("cannot write publication control: {error}"))
        })?;
        // Пути `ready` — операнды публикации, а не показ: вызывающий пишет по ним, и путь
        // длиннее 260 знаков без приставки не открылся бы.
        let encoded = match frame {
            Frame::Ready { .. } => encoded,
            _ => response_json(&encoded).into_owned(),
        };
        let mut output = std::io::stdout().lock();
        output
            .write_all(encoded.as_bytes())
            .and_then(|_| output.write_all(b"\n"))
            .and_then(|_| output.flush())
            .map_err(|error| {
                AppError::Runtime(format!("cannot flush publication control: {error}"))
            })
    }

    fn receive(&self, expected: Phase) -> Result<Decision, AppError> {
        let input = self
            .input
            .lock()
            .map_err(|_| AppError::Runtime("publication input lock poisoned".into()))?;
        let bytes = loop {
            // A queued Published decision takes precedence over a later signal.
            match input.try_recv() {
                Ok(result) => break result.map_err(AppError::Runtime)?,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(AppError::Runtime("publication control input closed".into()))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Some(cancellation) = crate::use_cases::interruption::SafePointCancel::noticed(
                &self.context,
                crate::use_cases::interruption::SafePoint::Named(
                    "while waiting for full-pull authorization",
                ),
            ) {
                return Err(if expected == Phase::Ready && !self.snapshot_only {
                    AppError::Runtime("publication outcome is unknown after cancellation while awaiting the publisher".into())
                } else {
                    cancellation.into_error()
                });
            }
            match input.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => break result.map_err(AppError::Runtime)?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(AppError::Runtime("publication control input closed".into()))
                }
            }
        };
        let reply: Reply = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Validation(format!("invalid publication control frame: {error}"))
        })?;
        let (version, nonce, phase, decision) = reply.into_parts();
        if version != VERSION || nonce != self.nonce || phase != expected {
            return Err(AppError::Validation(
                "publication control version, nonce or phase does not match".into(),
            ));
        }
        Ok(decision)
    }

    fn terminal<T: Serialize>(&self, result: &Envelope<T>) -> Result<(), AppError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::Runtime("publication state lock poisoned".into()))?;
        self.emit(Frame::Terminal {
            version: VERSION,
            nonce: &self.nonce,
            publication: state.publication,
            memory: state.memory,
            result,
        })
    }
}

impl FullPullControl for StdioControl {
    fn snapshot_only(&self) -> bool {
        self.snapshot_only
    }

    fn prepare_merge_baseline(&self) -> bool {
        self.prepare_merge_baseline
    }

    fn authorize(&self, executor: &FullPullExecutor<'_>) -> Result<(), AppError> {
        let kind = match executor.kind {
            ExecutorKind::Local => WireExecutorKind::Local,
            ExecutorKind::Attached => WireExecutorKind::Attached,
            ExecutorKind::Remote => WireExecutorKind::Remote,
        };
        self.emit::<Value>(Frame::Selected {
            version: VERSION,
            nonce: &self.nonce,
            provider: executor.provider,
            executor: Executor {
                kind,
                executable: executor.executable,
                endpoint: executor.endpoint,
            },
        })?;
        match self.receive(Phase::Selected)? {
            Decision::Continue => Ok(()),
            Decision::Reject => Err(AppError::Validation(
                "selected full-pull executor was rejected by the caller".into(),
            )),
            Decision::Published | Decision::Rejected => unreachable!("receive validates the phase"),
        }
    }

    fn publish(&self, ready: &FullPullReady<'_>) -> Result<(), AppError> {
        self.state
            .lock()
            .map_err(|_| AppError::Runtime("publication state lock poisoned".into()))?
            .publication = Publication::Unknown;
        let snapshot_warning = ready.snapshot_warning.map(response_text);
        self.emit::<Value>(Frame::Ready {
            version: VERSION,
            nonce: &self.nonce,
            command: ready.command.as_str(),
            requested_mode: ready.requested_mode,
            mode: ready.mode,
            extension: ready.extension,
            mode_reason: ready.mode_reason,
            provider_dispatched: ready.provider_dispatched,
            source_set: ready.source_set,
            target_path: ready.target,
            stage_path: ready.stage,
            snapshot: if ready.snapshot_prepared {
                Snapshot::Prepared
            } else {
                Snapshot::Unavailable
            },
            snapshot_warning: snapshot_warning.as_deref(),
            merge_baseline: ready.merge_baseline,
        })?;
        match self.receive(Phase::Ready)? {
            Decision::Published if self.snapshot_only => Err(AppError::Validation(
                "published is not allowed with --snapshot-only".into(),
            )),
            Decision::Published => {
                self.state
                    .lock()
                    .map_err(|_| AppError::Runtime("publication state lock poisoned".into()))?
                    .publication = Publication::Published;
                Ok(())
            }
            Decision::Rejected => {
                self.state
                    .lock()
                    .map_err(|_| AppError::Runtime("publication state lock poisoned".into()))?
                    .publication = Publication::NotPublished;
                Err(AppError::Validation(
                    "full-pull publication was rejected; baseline unchanged".into(),
                ))
            }
            Decision::Continue | Decision::Reject => unreachable!("receive validates the phase"),
        }
    }

    fn memory_finished(&self, outcome: MemoryCommit) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.memory = match outcome {
            MemoryCommit::Saved => Memory::Saved,
            MemoryCommit::NotSaved => Memory::NotSaved,
        };
    }
}

pub(crate) fn run(cli: &Cli, args: &DumpArgs, startup_refusal: Option<UseCaseError>) -> i32 {
    let cancellation = CancellationToken::new();
    let _signals = super::signal::CliSignalGuard::install(cancellation.clone());
    let context = ExecutionContext::cli(CommandName::Dump).with_cancellation(cancellation);
    let control = Arc::new(StdioControl::new(
        context.clone(),
        args.snapshot_only,
        args.prepare_merge_baseline,
    ));
    let outcome = execute(cli, args, startup_refusal, context, &control);
    let (terminal, exit_code) = match outcome {
        Ok((Some(result), code)) => (control.terminal(&result), code),
        Ok((None, code)) => return code,
        Err(error) => (
            control.terminal(&super::output::pre_dispatch_error_envelope("pull", &error)),
            error.exit_code(),
        ),
    };
    match terminal {
        Ok(()) => exit_code,
        Err(error) => {
            eprintln!("{error}");
            3
        }
    }
}

fn execute(
    cli: &Cli,
    args: &DumpArgs,
    startup_refusal: Option<UseCaseError>,
    context: ExecutionContext,
    control: &Arc<StdioControl>,
) -> Result<(Option<Envelope<Value>>, i32), UseCaseError> {
    if let Some(error) = startup_refusal {
        return Err(error);
    }
    let loaded = crate::app::load_cli_config(cli).map_err(|error| {
        let next = error.next();
        let error = UseCaseError::from(AppError::from(error));
        match next {
            Some(next) => error.with_next(next),
            None => error,
        }
    })?;
    let config = loaded.config;
    let config_path = crate::config::loader::resolve_primary_config_path(cli.config.as_deref())
        .map_err(|error| UseCaseError::from(AppError::from(error)))?;
    let command_line = crate::app::command_line_target(cli, Some(config_path), &config);
    let init_logging = || {
        crate::support::logging::init_action_logging(
            cli.log_level.as_deref().unwrap_or("info"),
            "json",
            false,
            &config.work_path,
            cli.dry_run,
        )
        .map_err(|error| UseCaseError::new(UseCaseErrorKind::Runtime, error.to_string()))
    };
    if !args.snapshot_only {
        init_logging()?;
    }
    let request = super::execute::map_dump_request(args, cli.dry_run)?;
    if config.format != SourceFormat::Designer || request.source_set.is_none() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "external publication requires DESIGNER format and one explicit source set",
        ));
    }
    if cli.clean_before_execution {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "external publication does not accept --clean-before-execution",
        ));
    }
    let stage = args.publication_stdio.as_ref().ok_or_else(|| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            "missing external publication stage",
        )
    })?;
    let stage = std::fs::canonicalize(stage).map_err(|error| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            format!("external publication stage must exist: {error}"),
        )
    })?;
    if args.snapshot_only {
        let base_dir = config
            .v8_connection()
            .file_infobase_dir(&config.base_path)
            .ok_or_else(|| {
                UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "cannot resolve the selected file infobase directory for the snapshot",
                )
            })?;
        let base_dir = crate::support::path::nearest_existing_canonical_path(&base_dir)
            .map_err(|error| UseCaseError::new(UseCaseErrorKind::Validation, error.to_string()))?;
        if stage.starts_with(&base_dir) || base_dir.starts_with(&stage) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "snapshot stage must not overlap the infobase directory",
            ));
        }
    }
    if !stage.is_dir() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "external publication stage must be a directory",
        ));
    }
    let original = if args.snapshot_only {
        crate::use_cases::dump_config::snapshot_source_path(&config, &request)
            .map_err(UseCaseError::from)?
    } else {
        let original = config
            .source_sets
            .iter()
            .find(|set| Some(set.name.as_str()) == request.source_set.as_deref())
            .ok_or_else(|| {
                UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "external full pull source set is not declared",
                )
            })?
            .root_in(&config.base_path);
        crate::support::path::nearest_existing_canonical_path(&original)
            .map_err(|error| UseCaseError::new(UseCaseErrorKind::Validation, error.to_string()))?
    };
    if stage.starts_with(&original)
        || original.starts_with(&stage)
        || crate::change_detection::runner_dirs::overlaps_a_runner_dir(
            &stage,
            crate::change_detection::runner_dirs::RunnerDirs::of(&config),
        )
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "external stage must not overlap source set, workPath or localPath",
        ));
    }
    if !cli.dry_run
        && std::fs::read_dir(&stage)
            .map_err(|error| UseCaseError::new(UseCaseErrorKind::Validation, error.to_string()))?
            .next()
            .is_some()
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "external publication stage must be empty",
        ));
    }
    if args.snapshot_only {
        init_logging()?;
    }
    let publication = Arc::new(ExternalFullPull::new(stage, control.clone()));
    let context = context
        .with_command_line(command_line)
        .with_external_full_pull(publication);
    let warnings: Vec<Warning> = loaded.warnings.into_iter().map(Warning::Plain).collect();
    let base_access = if args.snapshot_only {
        BaseAccess::ReadsWithLock
    } else {
        BaseAccess::Writes
    };
    // Both callbacks run synchronously. Keep the boundary notes for the terminal frame.
    let warnings = std::cell::RefCell::new(warnings);
    dispatch_with_workspace_boundary(
        &config,
        CommandName::Dump,
        base_access,
        cli.dry_run,
        |notes| {
            warnings
                .borrow_mut()
                .extend(notes.iter().map(|note| note.warning.clone()));
            Ok(())
        },
        || {
            let result = crate::use_cases::dump_config::execute(&context, &config, &request);
            let (mut envelope, exit_code) = match result {
                Ok(result) => (
                    Envelope::ok(
                        "pull",
                        result.duration_ms,
                        serde_json::to_value(result).map_err(serialization_error)?,
                    ),
                    0,
                ),
                Err(failure) => {
                    let error = failure.error;
                    let data =
                        serde_json::to_value(failure.payload).map_err(serialization_error)?;
                    (failure_envelope("pull", 0, data, &error), error.exit_code())
                }
            };
            envelope.follow_with(warnings.borrow().iter());
            if args.snapshot_only {
                // The mandatory base lock covers the terminal frame as well as the dump.
                let code = match control.terminal(&envelope) {
                    Ok(()) => exit_code,
                    Err(error) => {
                        eprintln!("{error}");
                        3
                    }
                };
                Ok((None, code))
            } else {
                Ok((Some(envelope), exit_code))
            }
        },
    )
    .map_err(|refusal| refusal.error)?
}

pub(crate) fn run_merge_baseline(
    cli: &Cli,
    args: &DumpArgs,
    id: uuid::Uuid,
    presenter: &crate::output::presenter::Presenter,
) -> i32 {
    let started = std::time::Instant::now();
    let outcome = (|| {
        if cli.dry_run || cli.clean_before_execution {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "working merge baseline checks and acceptance require a live generation read and cannot clean workPath",
            ));
        }
        let cancellation = CancellationToken::new();
        let _signals = super::signal::CliSignalGuard::install(cancellation.clone());
        let loaded = crate::app::load_cli_config(cli)
            .map_err(|error| UseCaseError::from(AppError::from(error)))?;
        let config = loaded.config;
        presenter.note_leading_warnings("pull", &loaded.warnings);
        let request = super::execute::map_dump_request(args, false)?;
        let config_path = crate::config::loader::resolve_primary_config_path(cli.config.as_deref())
            .map_err(|error| UseCaseError::from(AppError::from(error)))?;
        let command_line = crate::app::command_line_target(cli, Some(config_path), &config);
        let context = ExecutionContext::cli(CommandName::Dump)
            .with_cancellation(cancellation)
            .with_command_line(command_line);
        dispatch_with_workspace_boundary(
            &config,
            CommandName::Dump,
            BaseAccess::ReadsWithMemory,
            false,
            |notes| {
                for note in notes {
                    presenter.note_leading_warning("pull", &note.warning);
                }
                Ok(())
            },
            || {
                crate::use_cases::dump_config::snapshot_source_path(&config, &request)
                    .map_err(UseCaseError::from)?;
                let inventory =
                    crate::use_cases::source_inventory::SourceSetInventory::new(&config);
                let source = request
                    .source_set
                    .as_deref()
                    .and_then(|name| inventory.designer_context(name))
                    .ok_or_else(|| {
                        UseCaseError::new(
                            UseCaseErrorKind::Validation,
                            "working merge requires one declared configuration source set",
                        )
                    })?;
                crate::use_cases::merge_baseline::accept(
                    &context,
                    &config,
                    source,
                    id,
                    args.check_merge_baseline.is_some(),
                )
                .map_err(UseCaseError::from)
            },
        )
        .map_err(|refusal| refusal.error)?
    })();
    match outcome {
        Ok(result) => {
            presenter.print_bare(&format!(
                "working merge baseline {} for '{}' at generation {}",
                result.baseline, result.source_set, result.generation,
            ));
            presenter.print_envelope(&Envelope::ok(
                "pull",
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                result,
            ));
            0
        }
        Err(error) => {
            super::output::print_command_use_case_error(presenter, CommandName::Dump, &error);
            error.exit_code()
        }
    }
}

fn serialization_error(error: serde_json::Error) -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Runtime,
        format!("cannot serialize external full-pull result: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SCHEMA_PATH: &str = "docs/schemas/full-pull-publication.schema.json";

    #[derive(schemars::JsonSchema)]
    #[serde(untagged)]
    #[allow(dead_code, reason = "the schema names both directions of the protocol")]
    enum ProtocolMessage<'a> {
        Output(Frame<'a, Value>),
        Input(Reply),
    }

    fn generated_publication_schema() -> Value {
        let mut schema = crate::command_data::generated_schema(
            schemars::schema_for!(ProtocolMessage<'static>),
            "full-pull-publication",
        );
        schema["$id"] = json!(format!(
            "{}/full-pull-publication.schema.json",
            crate::command_data::REPOSITORY_RAW_SCHEMA_ROOT
        ));
        schema["title"] = json!("v8-runner external full-pull publication protocol");
        schema
    }

    #[test]
    fn generated_publication_schema_is_current() {
        let generated = crate::command_data::schema_json_pretty(&generated_publication_schema());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH);
        if std::env::var_os("UPDATE_PUBLICATION_SCHEMA").is_some() {
            std::fs::write(&path, &generated).expect("write publication schema");
        }
        assert_eq!(
            std::fs::read_to_string(path).expect("publication schema artifact"),
            generated,
            "{SCHEMA_PATH} is stale; rerun UPDATE_PUBLICATION_SCHEMA=1 cargo test generated_publication_schema_is_current"
        );
    }

    #[test]
    fn publication_schema_closes_each_phase_and_its_decisions() {
        let validator = jsonschema::validator_for(&generated_publication_schema()).expect("schema");
        let nonce = uuid::Uuid::new_v4().to_string();
        for (phase, decision) in [
            ("selected", "continue"),
            ("selected", "reject"),
            ("ready", "published"),
            ("ready", "rejected"),
        ] {
            let reply = json!({"version":1,"nonce":nonce,"phase":phase,"decision":decision});
            assert!(validator.is_valid(&reply), "valid reply: {reply}");
            assert!(serde_json::from_value::<Reply>(reply.clone()).is_ok());
            let mut changed = reply.clone();
            changed["other"] = json!(true);
            assert!(!validator.is_valid(&changed), "unknown field: {changed}");
            assert!(serde_json::from_value::<Reply>(changed).is_err());
            let mut changed = reply.clone();
            changed["version"] = json!(2);
            assert!(!validator.is_valid(&changed), "unknown version: {changed}");
            let mut changed = reply;
            changed["decision"] = json!(if phase == "ready" {
                "continue"
            } else {
                "published"
            });
            assert!(
                !validator.is_valid(&changed),
                "decision in wrong phase: {changed}"
            );
            assert!(serde_json::from_value::<Reply>(changed).is_err());
        }

        let result = Envelope::ok("pull", 0, Value::Null);
        let ready = serde_json::to_value::<Frame<'_, Value>>(Frame::Ready {
            version: VERSION,
            nonce: &nonce,
            command: "pull",
            requested_mode: crate::domain::dump::DumpMode::Full,
            mode: crate::domain::dump::ReportedDumpMode::Full,
            extension: Some("extension-name"),
            mode_reason: None,
            provider_dispatched: true,
            source_set: "extension-name",
            target_path: std::path::Path::new("sources"),
            stage_path: std::path::Path::new("stage"),
            snapshot: Snapshot::Prepared,
            snapshot_warning: None,
            merge_baseline: None,
        })
        .unwrap();
        assert!(validator.is_valid(&ready), "ready receipt: {ready}");
        for field in ["command", "requested_mode", "mode", "provider_dispatched"] {
            let mut invalid = ready.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(!validator.is_valid(&invalid), "missing {field}: {invalid}");
        }
        let terminal = serde_json::to_value(Frame::Terminal {
            version: VERSION,
            nonce: &nonce,
            publication: Publication::Published,
            memory: Memory::Saved,
            result: &result,
        })
        .expect("terminal frame");
        assert!(validator.is_valid(&terminal), "terminal frame: {terminal}");
        for pointer in ["/extra", "/result/extra"] {
            let mut invalid = terminal.clone();
            let (parent, field) = pointer.rsplit_once('/').expect("pointer field");
            invalid.pointer_mut(parent).expect("parent object")[field] = json!(true);
            assert!(!validator.is_valid(&invalid), "open object: {invalid}");
        }
    }
}
