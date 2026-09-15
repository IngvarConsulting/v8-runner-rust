use std::path::{Path, PathBuf};
use std::time::Instant;

use tracing::debug;

use crate::config::model::{AppConfig, SourceFormat};
use crate::domain::artifact::{ArtifactKind, ArtifactRef, ArtifactSet, ARTIFACT_ROLE_PLATFORM_LOG};
use crate::domain::artifacts::ArtifactBuildMode;
use crate::domain::capability::{Operation, Provider};
use crate::domain::execution::{ExecutionError, ExecutionOutcome, ExecutionStatus};
use crate::domain::load::{
    CompatibilityState, LoadExecutionMetadata, LoadMode, LoadResult, LoadTargetKind,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::extension_inventory::parse_extension_inventory;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, ExecutionInterruption, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::format_ibcmd_failure_details;
use crate::use_cases::interruption::{
    command_interruption_details, command_interruption_status,
    deferred_process_interruption_details, deferred_process_interruption_warning,
    interruption_before_safe_point_message,
};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::LoadRequest;
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};

const SUPPORTED_LOAD_ERROR: &str =
    "load currently supports only the Designer provider and format=DESIGNER";
const UNSUPPORTED_EXTERNAL_ARTIFACTS_ERROR: &str =
    "load currently supports only .cf and .cfe artifacts";
const UNSUPPORTED_UPDATE_MODE_ERROR: &str =
    "load --mode update is not supported; use --mode load or --mode merge";

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LoadRequest,
) -> UseCaseResult<LoadResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        mode = ?args.mode,
        artifact = args.artifact_path.as_str(),
        extension = args.extension.as_deref().unwrap_or("<none>"),
        "executing load use case"
    );
    run_load(context, config, args)
}

type LoadExecutionFailure = UseCaseFailure<LoadResult>;

#[derive(Debug, Clone)]
struct ResolvedLoadRequest {
    mode: LoadMode,
    artifact_path: PathBuf,
    artifact_type: ArtifactBuildMode,
    target_kind: LoadTargetKind,
    settings_path: Option<PathBuf>,
    extension: Option<String>,
    /// Name of the vendor configuration, when the caller stated it.
    vendor_name: Option<String>,
}

impl ResolvedLoadRequest {
    /// The name the platform needs before it will compare the target with its counterpart.
    ///
    /// An extension names itself; a configuration is named by its vendor counterpart, and the
    /// platform refuses the comparison without that name (measured on 8.3.27.2074). No name,
    /// no probe — and no guess either.
    fn comparison_name(&self) -> Option<&str> {
        match self.target_kind {
            LoadTargetKind::Configuration => self.vendor_name.as_deref(),
            LoadTargetKind::Extension => self.extension.as_deref(),
            LoadTargetKind::Unknown => None,
        }
    }
}

fn run_load(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LoadRequest,
) -> UseCaseResult<LoadResult> {
    let started = Instant::now();
    // One owner of the truth about the run: flipped where a platform process is actually
    // started, and carried into every payload instead of a constant `true`.
    let dispatched = false;
    let request_snapshot = request_snapshot_for_failure_payload(args);

    if let Some(error) = validate_supported_matrix(config) {
        return Err(LoadExecutionFailure::with_payload(
            error,
            empty_result(
                dispatched,
                args.mode,
                PathBuf::from(&args.artifact_path),
                request_snapshot.artifact_type,
                request_snapshot.target_kind,
                CompatibilityState::NotProbed,
                request_snapshot.extension,
                started,
                Some(SUPPORTED_LOAD_ERROR.to_owned()),
                None,
                false,
            ),
        ));
    }

    let resolved = match resolve_request(config, args) {
        Ok(resolved) => resolved,
        Err(error) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result(
                    dispatched,
                    args.mode,
                    PathBuf::from(&args.artifact_path),
                    request_snapshot.artifact_type,
                    request_snapshot.target_kind,
                    CompatibilityState::NotProbed,
                    request_snapshot.extension,
                    started,
                    Some(message),
                    None,
                    false,
                ),
            ));
        }
    };

    if let Some(interruption) = context.interruption() {
        let message = interruption_before_safe_point_message(context, interruption, "load probe");
        return Err(LoadExecutionFailure::with_payload(
            AppError::Runtime(message.clone()),
            interrupted_result_from_resolved(
                &resolved,
                CompatibilityState::NotProbed,
                started,
                interruption,
                message,
                None,
            ),
        ));
    }

    let mut utilities = PlatformUtilities::from_config(config);
    let selected = match crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        crate::domain::capability::Operation::Load,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let message = error.to_string();
            let mut result = empty_result_from_resolved(
                dispatched,
                &resolved,
                CompatibilityState::NotProbed,
                started,
                Some(message),
                None,
                false,
            );
            result.provider = Some(receipt);
            return Err(LoadExecutionFailure::with_payload(error, result));
        }
    };
    let receipt = selected.receipt.clone();
    let outcome = run_load_selected(
        context, config, args, started, dispatched, resolved, utilities, selected,
    );
    crate::use_cases::provider_selection::attach(outcome, &receipt)
}

#[allow(clippy::too_many_arguments)]
fn run_load_selected(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LoadRequest,
    started: Instant,
    mut dispatched: bool,
    resolved: ResolvedLoadRequest,
    mut utilities: PlatformUtilities,
    selected: crate::use_cases::provider_selection::SelectedProvider,
) -> UseCaseResult<LoadResult> {
    let Some(location) = selected.location else {
        return Err(UseCaseFailure::without_payload(
            crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::Load,
                selected.provider,
            ),
        ));
    };

    if args.dry_run {
        crate::use_cases::progress::log_live_stage(
            "load: preview",
            "[Load] preview only, nothing probed or applied",
        );
        // The next step is the compatibility probe, and the probe is a Designer run against
        // the infobase. A preview must not dispatch it, so the compatibility state stays
        // `not_probed` — a named case, not a guess — and nothing is reported as applied.
        let execution = ExecutionOutcome::new(ExecutionStatus::Succeeded)
            .with_payload(LoadExecutionMetadata {
                applied: false,
                target_kind: resolved.target_kind,
                compatibility_state: CompatibilityState::NotProbed,
                update_db_cfg_ran: false,
            })
            .with_diagnostics(vec![format!(
                "would load {} via {}; compatibility not probed and nothing applied",
                target_label(&resolved),
                location.path.display()
            )]);
        return Ok(LoadResult {
            provider: None,
            provider_dispatched: false,
            mode: resolved.mode,
            artifact_path: resolved.artifact_path,
            artifact_type: resolved.artifact_type,
            extension: resolved.extension,
            duration_ms: started.elapsed().as_millis() as u64,
            execution,
        });
    }

    log_live_stage(
        "load: compatibility probe",
        "[Конфигуратор] comparing infobase compatibility",
    );
    let probe_result = match probe_compatibility(
        context,
        config,
        location.path.as_path(),
        &mut utilities,
        &resolved,
    ) {
        Ok(result) => result,
        Err((error, platform_log_path)) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result_from_resolved(
                    dispatched,
                    &resolved,
                    CompatibilityState::NotProbed,
                    started,
                    Some(message),
                    platform_log_path,
                    false,
                ),
            ));
        }
    };

    let compatibility_state = probe_result.state;
    dispatched = probe_result.dispatched;
    let probe_log_path = probe_result.platform_log_path;
    let probe_evidence = probe_result.diagnostic;
    if let Some(error) =
        validate_probe_mode_compatibility(&resolved, compatibility_state, probe_evidence.as_deref())
    {
        let message = error.to_string();
        return Err(LoadExecutionFailure::with_payload(
            error,
            empty_result_from_resolved(
                dispatched,
                &resolved,
                compatibility_state,
                started,
                Some(message),
                probe_log_path,
                false,
            ),
        ));
    }

    dispatched = true;
    let apply_dsl = match build_designer_dsl(
        context,
        config,
        location.path.as_path(),
        utilities.runner_for(UtilityType::V8),
        match resolved.mode {
            LoadMode::Load => "load",
            LoadMode::Merge => "merge",
            LoadMode::Update => unreachable!("update mode is rejected during validation"),
        },
        InterruptionSafetyClass::CriticalNonAbortable,
    ) {
        Ok(dsl) => dsl,
        Err((error, platform_log_path)) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result_from_resolved(
                    dispatched,
                    &resolved,
                    compatibility_state,
                    started,
                    Some(message),
                    platform_log_path.or(probe_log_path),
                    false,
                ),
            ));
        }
    };

    let apply_stage = match resolved.mode {
        LoadMode::Load => "load: apply",
        LoadMode::Merge => "load: merge",
        LoadMode::Update => unreachable!("update mode is rejected during validation"),
    };
    log_live_stage(apply_stage, "[Конфигуратор] applying artifact");
    let apply_result = match resolved.mode {
        LoadMode::Load => apply_dsl
            .load_cfg(&resolved.artifact_path, resolved.extension.as_deref())
            .map_err(|error| (AppError::from(error), None)),
        LoadMode::Merge => apply_dsl
            .merge_cfg(
                &resolved.artifact_path,
                resolved
                    .settings_path
                    .as_deref()
                    .expect("merge settings were validated"),
                resolved.extension.as_deref(),
            )
            .map_err(|error| (AppError::from(error), None)),
        LoadMode::Update => unreachable!("update mode is rejected during validation"),
    };

    let apply_result = match apply_result {
        Ok(result) => result,
        Err((error, platform_log_path)) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result_from_resolved(
                    dispatched,
                    &resolved,
                    compatibility_state,
                    started,
                    Some(message),
                    platform_log_path.or(probe_log_path),
                    false,
                ),
            ));
        }
    };

    let apply_action = match resolved.mode {
        LoadMode::Load => "load",
        LoadMode::Merge => "merge",
        LoadMode::Update => unreachable!("update mode is rejected during validation"),
    };

    if let Err(error) = ensure_platform_success(apply_action, &resolved, &apply_result) {
        let message = error.to_string();
        return Err(LoadExecutionFailure::with_payload(
            error,
            empty_result_from_resolved(
                dispatched,
                &resolved,
                compatibility_state,
                started,
                Some(message),
                apply_result.platform_log_path.or(probe_log_path),
                false,
            ),
        ));
    }

    if let Some(interruption) = context.interruption() {
        let message =
            interruption_before_safe_point_message(context, interruption, "update_db_cfg");
        return Err(LoadExecutionFailure::with_payload(
            AppError::Runtime(message.clone()),
            interrupted_result_from_resolved(
                &resolved,
                compatibility_state,
                started,
                interruption,
                message,
                apply_result.platform_log_path.or(probe_log_path),
            ),
        ));
    }

    let update_dsl = match build_designer_dsl(
        context,
        config,
        location.path.as_path(),
        utilities.runner_for(UtilityType::V8),
        "update-db-cfg",
        InterruptionSafetyClass::CriticalNonAbortable,
    ) {
        Ok(dsl) => dsl,
        Err((error, platform_log_path)) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result_from_resolved(
                    dispatched,
                    &resolved,
                    compatibility_state,
                    started,
                    Some(message),
                    platform_log_path
                        .or(apply_result.platform_log_path)
                        .or(probe_log_path),
                    false,
                ),
            ));
        }
    };

    log_live_stage(
        "load: update_db_cfg",
        "[Конфигуратор] updating database configuration",
    );
    let update_result = update_dsl
        .update_db_cfg(resolved.extension.as_deref())
        .map_err(AppError::from);

    let update_result = match update_result {
        Ok(result) => result,
        Err(error) => {
            let message = error.to_string();
            return Err(LoadExecutionFailure::with_payload(
                error,
                empty_result_from_resolved(
                    dispatched,
                    &resolved,
                    compatibility_state,
                    started,
                    Some(message),
                    apply_result.platform_log_path.or(probe_log_path),
                    false,
                ),
            ));
        }
    };

    if let Err(error) = ensure_platform_success("update_db_cfg", &resolved, &update_result) {
        let message = error.to_string();
        return Err(LoadExecutionFailure::with_payload(
            error,
            empty_result_from_resolved(
                dispatched,
                &resolved,
                compatibility_state,
                started,
                Some(message),
                update_result
                    .platform_log_path
                    .or(apply_result.platform_log_path)
                    .or(probe_log_path),
                false,
            ),
        ));
    }

    let deferred_warnings = [
        deferred_interruption_warning("apply", &apply_result),
        deferred_interruption_warning("update_db_cfg", &update_result),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let deferred_interruptions = [
        deferred_process_interruption_details(
            "apply",
            "apply completed successfully",
            &apply_result,
        ),
        deferred_process_interruption_details(
            "update_db_cfg",
            "update_db_cfg completed successfully",
            &update_result,
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let mut execution =
        ExecutionOutcome::new(ExecutionStatus::Succeeded).with_payload(LoadExecutionMetadata {
            applied: true,
            target_kind: resolved.target_kind,
            compatibility_state,
            update_db_cfg_ran: true,
        });
    if !deferred_warnings.is_empty() {
        execution = execution.with_diagnostics(deferred_warnings);
    }
    if !deferred_interruptions.is_empty() {
        execution = execution.with_interruptions(deferred_interruptions);
    }
    Ok(LoadResult {
        provider: None,
        provider_dispatched: true,
        mode: resolved.mode,
        artifact_path: resolved.artifact_path,
        artifact_type: resolved.artifact_type,
        extension: resolved.extension,
        duration_ms: started.elapsed().as_millis() as u64,
        execution: with_platform_log_artifact(
            execution,
            update_result
                .platform_log_path
                .or(apply_result.platform_log_path)
                .or(probe_log_path),
        ),
    })
}

struct ProbeResult {
    state: CompatibilityState,
    /// Whether asking actually started a platform process. `provider_dispatched` on the wire
    /// must be the truth about the run, not a constant.
    dispatched: bool,
    platform_log_path: Option<PathBuf>,
    /// What the platform said about a probe that did not run — carried, never interpreted.
    diagnostic: Option<String>,
}

fn probe_compatibility(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    utilities: &mut PlatformUtilities,
    resolved: &ResolvedLoadRequest,
) -> Result<ProbeResult, (AppError, Option<PathBuf>)> {
    // An extension needs no comparison at all. The decision only asks whether there is
    // something to merge into, and the infobase answers that structurally: `ibcmd config
    // extension list` prints keyed records whose field names and values are the same in every
    // interface language. Comparing the extension with its database copy told us nothing more
    // and told it in prose.
    if resolved.target_kind == LoadTargetKind::Extension {
        let (state, diagnostic, dispatched) =
            match installed_extension_state(context, config, utilities, resolved) {
                ExtensionPresence::Absent => (CompatibilityState::Absent, None, true),
                ExtensionPresence::Present => (CompatibilityState::Supported, None, true),
                ExtensionPresence::NotEstablished(reason, dispatched) => {
                    (CompatibilityState::NotEstablished, Some(reason), dispatched)
                }
            };
        return Ok(ProbeResult {
            state,
            dispatched,
            platform_log_path: None,
            diagnostic,
        });
    }
    let Some(comparison_name) = resolved.comparison_name() else {
        // Nothing to ask with: the platform will not compare a configuration without the
        // vendor configuration's name. Saying "not asked" is the honest answer; guessing the
        // support state from the refusal text is what ADR-0029 forbids.
        return Ok(ProbeResult {
            state: CompatibilityState::NotProbed,
            dispatched: false,
            platform_log_path: None,
            diagnostic: None,
        });
    };
    let report_dir = config.work_path.join("load-probe");
    std::fs::create_dir_all(&report_dir).map_err(|error| {
        (
            AppError::Runtime(format!("failed to prepare load probe dir: {error}")),
            None,
        )
    })?;
    let report_file = report_dir.join(match resolved.target_kind {
        LoadTargetKind::Configuration => "configuration-compare.txt",
        LoadTargetKind::Extension => "extension-compare.txt",
        LoadTargetKind::Unknown => "unknown-compare.txt",
    });

    let dsl = build_designer_dsl(
        context,
        config,
        binary,
        utilities.runner_for(UtilityType::V8),
        "probe",
        InterruptionSafetyClass::GracefulThenKill,
    )?;
    let result = match resolved.target_kind {
        LoadTargetKind::Configuration => dsl.compare_cfg(
            "MainConfiguration",
            None,
            "VendorConfiguration",
            Some(comparison_name),
            &report_file,
        ),
        LoadTargetKind::Extension => dsl.compare_cfg(
            "ExtensionConfiguration",
            Some(comparison_name),
            "ExtensionDBConfiguration",
            Some(comparison_name),
            &report_file,
        ),
        LoadTargetKind::Unknown => unreachable!("unknown targets are rejected during validation"),
    }
    .map_err(|error| (AppError::from(error), None))?;

    // The whole classification: the comparison either ran or it did not. Exit zero is the
    // platform's own guarantee, and exactly then it writes the comparison report; every other
    // outcome leaves the state unestablished, whatever sentence the log carries.
    let state = if result.process.exit_code == 0 {
        CompatibilityState::Supported
    } else {
        CompatibilityState::NotEstablished
    };
    let diagnostic = probe_evidence(&result);
    Ok(ProbeResult {
        state,
        dispatched: true,
        platform_log_path: result.platform_log_path,
        diagnostic,
    })
}

enum ExtensionPresence {
    Present,
    Absent,
    /// The list could not be read. Asked and not proven, so no change is permitted. The flag
    /// says whether a platform process was started before the attempt gave up.
    NotEstablished(String, bool),
}

/// Asks the infobase whether the extension is installed, by its own keyed list.
fn installed_extension_state(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    resolved: &ResolvedLoadRequest,
) -> ExtensionPresence {
    let Some(name) = resolved.extension.as_deref() else {
        return ExtensionPresence::NotEstablished("the extension is not named".to_owned(), false);
    };
    let connection = match IbcmdConnection::from_infobase(&config.infobase) {
        Ok(connection) => connection,
        Err(error) => return ExtensionPresence::NotEstablished(error.to_string(), false),
    };
    let binary = match utilities.locate(UtilityType::Ibcmd) {
        Ok(location) => location.path,
        Err(error) => return ExtensionPresence::NotEstablished(error.to_string(), false),
    };
    let dsl = IbcmdDsl::new(binary, connection, utilities.runner_for(UtilityType::Ibcmd))
        .with_execution_policy(
            context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        );
    let result = match dsl.infobase_extension_list() {
        Ok(result) => result,
        // The spawn itself failed, so nothing ran.
        Err(error) => return ExtensionPresence::NotEstablished(error.to_string(), false),
    };
    if result.process.exit_code != 0 {
        return ExtensionPresence::NotEstablished(
            format!(
                "reading the extension list exited with {}",
                result.process.exit_code
            ),
            true,
        );
    }
    match parse_extension_inventory(&result.process.stdout) {
        Ok(extensions) => {
            if extensions.iter().any(|extension| extension.name == name) {
                ExtensionPresence::Present
            } else {
                ExtensionPresence::Absent
            }
        }
        Err(error) => ExtensionPresence::NotEstablished(error, true),
    }
}

/// The platform's own words about a probe that did not run, kept as evidence for a human.
///
/// Evidence is not a decision: nothing reads this back. It exists so the caller can see which
/// sentence the runner deliberately refused to interpret.
fn probe_evidence(result: &PlatformCommandResult) -> Option<String> {
    if result.process.exit_code == 0 {
        return None;
    }
    let log = result.platform_log.as_deref()?;
    let text = log
        .strip_prefix('\u{feff}')
        .unwrap_or(log)
        .replace("\r\n", "\n");
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// The whole decision, enumerated: target kind, requested mode, and what was established.
///
/// No default-permit arm (ADR-0023, point 6): a new target kind, mode or state cannot slip
/// through as "allowed" by falling into a wildcard. Nothing here reads a platform message —
/// `evidence` only travels into the refusal text so a human can see what was not interpreted.
fn validate_probe_mode_compatibility(
    resolved: &ResolvedLoadRequest,
    state: CompatibilityState,
    evidence: Option<&str>,
) -> Option<AppError> {
    use CompatibilityState::{Absent, NotEstablished, NotProbed, Supported};
    use LoadMode::{Load, Merge, Update};
    use LoadTargetKind::{Configuration, Extension, Unknown};

    let unproven = || {
        Some(AppError::Validation(format!(
            "{} state was asked and not established, so nothing is changed{}",
            target_label(resolved),
            evidence
                .map(|line| format!(". The platform said: {line}"))
                .unwrap_or_default()
        )))
    };

    match (resolved.target_kind, resolved.mode, state) {
        (_, Update, _) => Some(AppError::Validation(
            UNSUPPORTED_UPDATE_MODE_ERROR.to_owned(),
        )),
        // Asked and not proven permits no change, in either mode and for either target: the
        // fail-closed rule carried from ADR-0023. An unreadable extension list or an
        // infobase that will not open stops a load too.
        (Configuration | Extension, Load | Merge, NotEstablished) => unproven(),
        // An extension the infobase does not list is a first installation; merging into
        // nothing is the caller's mistake.
        (Extension, Load, Absent) => None,
        (Extension, Merge, Absent) => Some(AppError::Validation(format!(
            "{} is absent from the infobase; use --mode load for the first installation",
            target_label(resolved)
        ))),
        // A listed extension may be replaced wholesale or merged into. Both are legitimate and
        // which one is right is the caller's decision.
        (Extension, Load | Merge, Supported) => None,
        // Unreachable today — an extension is always asked — but a refusal is the safe answer
        // if that ever changes, and it keeps the matrix free of a permitting wildcard.
        (Extension, Load, NotProbed) => None,
        (Extension, Merge, NotProbed) => unproven(),
        // A configuration proven to be on support would silently lose that support to a full
        // load, so the caller is steered to the mode that preserves it.
        (Configuration, Load, Supported) => Some(AppError::Validation(format!(
            "{} is already compatible with merge; use --mode merge instead",
            target_label(resolved)
        ))),
        // Nobody asked, because the platform will not compare a configuration without naming
        // its vendor counterpart. That is not an unknown answer: `--mode load` is the caller's
        // stated fact, and a first installation is never blocked by an unasked question.
        (Configuration, Load, NotProbed) => None,
        // A configuration has no list to be absent from; the state is about support only.
        (Configuration, Load | Merge, Absent) => unproven(),
        (Configuration, Merge, Supported) => None,
        (Configuration, Merge, NotProbed) => Some(AppError::Validation(format!(
            "{} support state cannot be asked without the vendor configuration name; pass \
             --vendor-name to merge, or use --mode load for a first installation",
            target_label(resolved)
        ))),
        (Unknown, _, _) => unproven(),
    }
}

fn validate_supported_matrix(config: &AppConfig) -> Option<AppError> {
    if config.selected_provider(Operation::Load) == Provider::Designer
        && config.format == SourceFormat::Designer
    {
        None
    } else {
        Some(AppError::Validation(SUPPORTED_LOAD_ERROR.to_owned()))
    }
}

fn request_snapshot_for_failure_payload(args: &LoadRequest) -> ResolvedLoadRequest {
    let artifact_type =
        infer_artifact_type(&args.artifact_path).unwrap_or(ArtifactBuildMode::Unknown);
    let extension = trim_optional(args.extension.clone());
    let target_kind = match artifact_type {
        ArtifactBuildMode::ExtensionCfe => LoadTargetKind::Extension,
        ArtifactBuildMode::ConfigurationCf => LoadTargetKind::Configuration,
        ArtifactBuildMode::ExternalDataProcessorEpf | ArtifactBuildMode::ExternalReportErf => {
            LoadTargetKind::Unknown
        }
        ArtifactBuildMode::Unknown => LoadTargetKind::Unknown,
    };

    ResolvedLoadRequest {
        mode: args.mode,
        vendor_name: args.vendor_name.clone(),
        artifact_path: PathBuf::from(&args.artifact_path),
        artifact_type,
        target_kind,
        settings_path: None,
        extension,
    }
}

fn resolve_request(
    config: &AppConfig,
    args: &LoadRequest,
) -> Result<ResolvedLoadRequest, AppError> {
    if args.mode == LoadMode::Update {
        return Err(AppError::Validation(
            UNSUPPORTED_UPDATE_MODE_ERROR.to_owned(),
        ));
    }

    let artifact_path = resolve_existing_file(config, &args.artifact_path, "--path")?;
    let artifact_type = infer_artifact_type(&args.artifact_path)
        .ok_or_else(|| AppError::Validation(UNSUPPORTED_EXTERNAL_ARTIFACTS_ERROR.to_owned()))?;
    if matches!(
        artifact_type,
        ArtifactBuildMode::ExternalDataProcessorEpf | ArtifactBuildMode::ExternalReportErf
    ) {
        return Err(AppError::Validation(
            UNSUPPORTED_EXTERNAL_ARTIFACTS_ERROR.to_owned(),
        ));
    }

    let extension = trim_optional(args.extension.clone());
    let settings_path = match args.mode {
        LoadMode::Merge => Some(resolve_existing_file(
            config,
            args.settings_path.as_deref().ok_or_else(|| {
                AppError::Validation("load --mode merge requires --settings <file>".to_owned())
            })?,
            "--settings",
        )?),
        LoadMode::Load => {
            if args.settings_path.is_some() {
                return Err(AppError::Validation(
                    "--settings is supported only with --mode merge".to_owned(),
                ));
            }
            None
        }
        LoadMode::Update => None,
    };

    let (target_kind, extension) = match artifact_type {
        ArtifactBuildMode::ConfigurationCf => {
            if extension.is_some() {
                return Err(AppError::Validation(
                    ".cf artifacts do not support --extension".to_owned(),
                ));
            }
            (LoadTargetKind::Configuration, None)
        }
        ArtifactBuildMode::ExtensionCfe => {
            let extension = extension.ok_or_else(|| {
                AppError::Validation(".cfe artifacts require --extension <name>".to_owned())
            })?;
            (LoadTargetKind::Extension, Some(extension))
        }
        ArtifactBuildMode::ExternalDataProcessorEpf | ArtifactBuildMode::ExternalReportErf => {
            unreachable!("external artifacts are rejected above")
        }
        ArtifactBuildMode::Unknown => unreachable!("unknown artifacts are rejected above"),
    };

    Ok(ResolvedLoadRequest {
        mode: args.mode,
        artifact_path,
        artifact_type,
        target_kind,
        settings_path,
        extension,
        vendor_name: args.vendor_name.clone(),
    })
}

fn resolve_existing_file(
    config: &AppConfig,
    raw_path: &str,
    flag: &str,
) -> Result<PathBuf, AppError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(AppError::Validation(format!(
            "{flag} requires a non-empty file path"
        )));
    }
    let candidate = PathBuf::from(trimmed);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        config.base_path.join(candidate)
    };
    if !candidate.exists() {
        return Err(AppError::Validation(format!(
            "{flag} file does not exist: {}",
            candidate.display()
        )));
    }
    if !candidate.is_file() {
        return Err(AppError::Validation(format!(
            "{flag} must point to a file: {}",
            candidate.display()
        )));
    }
    std::fs::canonicalize(&candidate).map_err(|error| {
        AppError::Runtime(format!(
            "failed to canonicalize '{}': {error}",
            candidate.display()
        ))
    })
}

fn infer_artifact_type(raw_path: &str) -> Option<ArtifactBuildMode> {
    let extension = Path::new(raw_path)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    match extension.as_str() {
        "cf" => Some(ArtifactBuildMode::ConfigurationCf),
        "cfe" => Some(ArtifactBuildMode::ExtensionCfe),
        "epf" => Some(ArtifactBuildMode::ExternalDataProcessorEpf),
        "erf" => Some(ArtifactBuildMode::ExternalReportErf),
        _ => None,
    }
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn build_designer_dsl<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &'a dyn ProcessRunner,
    action: &str,
    safety: InterruptionSafetyClass,
) -> Result<DesignerDsl<'a>, (AppError, Option<PathBuf>)> {
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        (
            AppError::Runtime(format!("failed to create platform logs dir: {error}")),
            None,
        )
    })?;
    let log_file = log_dir.join(format!("load-{action}.log"));
    Ok(DesignerDsl::new(
        binary.to_path_buf(),
        config.v8_connection(),
        runner,
        Some(log_file),
    )
    .with_execution_policy(context.process_policy(safety, None)))
}

fn ensure_platform_success(
    action: &str,
    resolved: &ResolvedLoadRequest,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    if result.process.exit_code == 0 {
        return Ok(());
    }
    Err(AppError::Platform(format_ibcmd_failure_details(
        action,
        match resolved.target_kind {
            LoadTargetKind::Configuration => "configuration",
            LoadTargetKind::Extension => "extension",
            LoadTargetKind::Unknown => "unknown",
        },
        resolved.extension.as_deref().unwrap_or("main"),
        result.process.exit_code,
        &result.process.stdout,
        &result.process.stderr,
        result.platform_log.as_deref(),
        result.platform_log_path.as_deref(),
    )))
}

fn target_label(resolved: &ResolvedLoadRequest) -> String {
    match resolved.target_kind {
        LoadTargetKind::Configuration => "configuration".to_owned(),
        LoadTargetKind::Extension => format!(
            "extension '{}'",
            resolved.extension.as_deref().unwrap_or("<unknown>")
        ),
        LoadTargetKind::Unknown => "unknown".to_owned(),
    }
}

fn interrupted_result_from_resolved(
    resolved: &ResolvedLoadRequest,
    compatibility_state: CompatibilityState,
    started: Instant,
    interruption: ExecutionInterruption,
    message: String,
    platform_log_path: Option<PathBuf>,
) -> LoadResult {
    LoadResult {
        provider: None,
        provider_dispatched: true,
        mode: resolved.mode,
        artifact_path: resolved.artifact_path.clone(),
        artifact_type: resolved.artifact_type,
        extension: resolved.extension.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
        execution: with_platform_log_artifact(
            ExecutionOutcome::new(command_interruption_status(interruption))
                .with_diagnostics(vec![message.clone()])
                .with_errors(vec![ExecutionError::new(
                    "artifact_load_interrupted",
                    message.clone(),
                )])
                .with_interruptions(vec![command_interruption_details(
                    interruption,
                    "update_db_cfg_safe_point",
                    message,
                )])
                .with_payload(LoadExecutionMetadata {
                    applied: true,
                    target_kind: resolved.target_kind,
                    compatibility_state,
                    update_db_cfg_ran: false,
                }),
            platform_log_path,
        ),
    }
}

fn deferred_interruption_warning(action: &str, result: &PlatformCommandResult) -> Option<String> {
    deferred_process_interruption_warning(&format!("{action} completed successfully"), result)
}

fn empty_result_from_resolved(
    provider_dispatched: bool,
    resolved: &ResolvedLoadRequest,
    compatibility_state: CompatibilityState,
    started: Instant,
    message: Option<String>,
    platform_log_path: Option<PathBuf>,
    update_db_cfg_ran: bool,
) -> LoadResult {
    empty_result(
        provider_dispatched,
        resolved.mode,
        resolved.artifact_path.clone(),
        resolved.artifact_type,
        resolved.target_kind,
        compatibility_state,
        resolved.extension.clone(),
        started,
        message,
        platform_log_path,
        update_db_cfg_ran,
    )
}

fn empty_result(
    provider_dispatched: bool,
    mode: LoadMode,
    artifact_path: PathBuf,
    artifact_type: ArtifactBuildMode,
    target_kind: LoadTargetKind,
    compatibility_state: CompatibilityState,
    extension: Option<String>,
    started: Instant,
    message: Option<String>,
    platform_log_path: Option<PathBuf>,
    update_db_cfg_ran: bool,
) -> LoadResult {
    let error_message = message
        .clone()
        .unwrap_or_else(|| "artifact load failed".to_owned());
    LoadResult {
        provider: None,
        provider_dispatched,
        mode,
        artifact_path,
        artifact_type,
        extension,
        duration_ms: started.elapsed().as_millis() as u64,
        execution: with_platform_log_artifact(
            ExecutionOutcome::new(ExecutionStatus::Failed)
                .with_errors(vec![ExecutionError::new(
                    "artifact_load_failed",
                    error_message,
                )])
                .with_payload(LoadExecutionMetadata {
                    applied: false,
                    target_kind,
                    compatibility_state,
                    update_db_cfg_ran,
                }),
            platform_log_path,
        ),
    }
}

fn with_platform_log_artifact(
    execution: ExecutionOutcome<LoadExecutionMetadata>,
    platform_log_path: Option<PathBuf>,
) -> ExecutionOutcome<LoadExecutionMetadata> {
    let Some(platform_log_path) = platform_log_path else {
        return execution;
    };
    let mut artifacts = ArtifactSet::default();
    artifacts.push(
        ArtifactRef::new(ArtifactKind::PlatformLog, platform_log_path)
            .with_role(ARTIFACT_ROLE_PLATFORM_LOG),
    );
    execution.with_artifacts(artifacts)
}

#[cfg(test)]
mod tests {
    use super::{execute, resolve_request, ResolvedLoadRequest};
    use crate::config::model::{
        AppConfig, BuildConfig, PlatformToolConfig, SourceFormat, TestsConfig, ToolsConfig,
    };
    use crate::domain::artifacts::ArtifactBuildMode;
    use crate::domain::execution::ExecutionStatus;
    use crate::domain::load::{
        CompatibilityState, LoadExecutionMetadata, LoadMode, LoadResult, LoadTargetKind,
    };
    use crate::platform::process::ProcessResult;
    use crate::platform::result::PlatformCommandResult;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::LoadRequest;
    use crate::use_cases::result::UseCaseErrorKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn load_payload(result: &LoadResult) -> &LoadExecutionMetadata {
        result.execution.payload.as_ref().expect("payload")
    }

    fn load_message(result: &LoadResult) -> &str {
        result
            .execution
            .errors
            .first()
            .map(|error| error.message.as_str())
            .or_else(|| result.execution.diagnostics.first().map(String::as_str))
            .expect("message")
    }

    fn probe_result(
        stdout: &str,
        stderr: &str,
        platform_log: Option<&str>,
        platform_log_read_error: Option<&str>,
    ) -> PlatformCommandResult {
        PlatformCommandResult {
            process: ProcessResult {
                exit_code: 19,
                stdout: stdout.to_owned(),
                stderr: stderr.to_owned(),
                interruption: None,
            },
            platform_log_path: None,
            platform_log: platform_log.map(str::to_owned),
            platform_log_read_error: platform_log_read_error.map(str::to_owned),
        }
    }

    #[test]
    fn a_probe_that_ran_is_the_only_proof_of_support() {
        // The whole classification, and the reason the four prose tests that used to stand
        // here are gone (ADR-0029): exit zero means the comparison ran, and nothing else is
        // guaranteed. The platform's sentence is carried as evidence, never read.
        let ran = PlatformCommandResult {
            process: ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            },
            platform_log_path: None,
            platform_log: None,
            platform_log_read_error: None,
        };
        assert!(super::probe_evidence(&ran).is_none());

        let refused = probe_result(
            "",
            "",
            Some("\u{feff}Конфигурация 'Конфигурация поставщика' недоступна\r\n"),
            None,
        );
        assert_eq!(
            super::probe_evidence(&refused).as_deref(),
            Some("Конфигурация 'Конфигурация поставщика' недоступна"),
            "the platform's words travel as evidence, with BOM and CRLF stripped"
        );
    }

    /// Матрица «цель — режим — состояние» перебирается целиком: у каждой пары есть
    /// названный исход, и ни одно неустановленное состояние не разрешает изменение.
    /// Разрешающая ветка по умолчанию провалила бы именно этот перебор.
    #[test]
    fn the_compatibility_matrix_answers_every_combination_and_never_permits_an_unproven_one() {
        use CompatibilityState::{Absent, NotEstablished, NotProbed, Supported};
        use LoadMode::{Load, Merge, Update};
        use LoadTargetKind::{Configuration, Extension, Unknown};

        let states = [Supported, Absent, NotEstablished, NotProbed];
        let modes = [Load, Merge, Update];
        let kinds = [Configuration, Extension, Unknown];

        for kind in kinds {
            for mode in modes {
                for state in states {
                    let resolved = ResolvedLoadRequest {
                        mode,
                        artifact_path: PathBuf::from("dist/main.cf"),
                        artifact_type: ArtifactBuildMode::ConfigurationCf,
                        target_kind: kind,
                        settings_path: None,
                        extension: None,
                        vendor_name: None,
                    };
                    let verdict = super::validate_probe_mode_compatibility(&resolved, state, None);

                    if matches!(state, NotEstablished) || matches!(kind, Unknown) {
                        assert!(
                            verdict.is_some(),
                            "{kind:?}/{mode:?}/{state:?} must refuse: an unproven state permits no change"
                        );
                    }
                    if matches!(mode, Update) {
                        assert!(
                            verdict.is_some(),
                            "{kind:?}/{mode:?}/{state:?} must refuse: update mode is not supported"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn an_unestablished_state_refuses_a_merge_and_lets_a_first_load_through() {
        let configuration = ResolvedLoadRequest {
            mode: LoadMode::Load,
            artifact_path: PathBuf::from("dist/main.cf"),
            artifact_type: ArtifactBuildMode::ConfigurationCf,
            target_kind: LoadTargetKind::Configuration,
            settings_path: None,
            extension: None,
            vendor_name: None,
        };

        // Asked and not proven permits no change, in either mode (ADR-0029, point 5).
        assert!(
            super::validate_probe_mode_compatibility(
                &configuration,
                CompatibilityState::NotEstablished,
                Some("Vendor configuration is not available"),
            )
            .is_some(),
            "an unproven state must not permit a load either"
        );
        // A question nobody could ask is not an unknown answer: the caller's stated mode is
        // the fact, and `--mode load` says "install or replace".
        assert!(
            super::validate_probe_mode_compatibility(
                &configuration,
                CompatibilityState::NotProbed,
                None,
            )
            .is_none(),
            "a first installation of a configuration is never blocked by an unasked question"
        );
        // A proven absence is the first installation of an extension.
        assert!(super::validate_probe_mode_compatibility(
            &ResolvedLoadRequest {
                target_kind: LoadTargetKind::Extension,
                extension: Some("SalesAddon".to_owned()),
                ..configuration.clone()
            },
            CompatibilityState::Absent,
            None,
        )
        .is_none());

        let merge = ResolvedLoadRequest {
            mode: LoadMode::Merge,
            ..configuration.clone()
        };
        let refusal = super::validate_probe_mode_compatibility(
            &merge,
            CompatibilityState::NotEstablished,
            Some("Vendor configuration is not available"),
        )
        .expect("merge must be refused on an unproven state")
        .to_string();
        assert!(refusal.contains("not established"), "{refusal}");
        assert!(
            refusal.contains("Vendor configuration is not available"),
            "the evidence reaches the caller: {refusal}"
        );

        let refusal =
            super::validate_probe_mode_compatibility(&merge, CompatibilityState::NotProbed, None)
                .expect("merge without a name must be refused")
                .to_string();
        assert!(
            refusal.contains("--vendor-name"),
            "the refusal names the missing input: {refusal}"
        );

        assert!(super::validate_probe_mode_compatibility(
            &merge,
            CompatibilityState::Supported,
            None
        )
        .is_none());
        let refusal = super::validate_probe_mode_compatibility(
            &configuration,
            CompatibilityState::Supported,
            None,
        )
        .expect("a proven support state steers a load to merge")
        .to_string();
        assert!(refusal.contains("--mode merge"), "{refusal}");
    }

    #[test]
    fn a_configuration_without_a_vendor_name_is_never_asked() {
        // The platform refuses to compare a configuration with its vendor counterpart unless
        // the counterpart is named (measured on 8.3.27.2074), so there is no question to ask
        // and `NotProbed` is the honest answer.
        let configuration = ResolvedLoadRequest {
            mode: LoadMode::Merge,
            artifact_path: PathBuf::from("dist/main.cf"),
            artifact_type: ArtifactBuildMode::ConfigurationCf,
            target_kind: LoadTargetKind::Configuration,
            settings_path: None,
            extension: None,
            vendor_name: None,
        };
        assert_eq!(configuration.comparison_name(), None);
        assert_eq!(
            ResolvedLoadRequest {
                vendor_name: Some("УправлениеТорговлей".to_owned()),
                ..configuration.clone()
            }
            .comparison_name(),
            Some("УправлениеТорговлей")
        );
        // An extension names itself, so it is always askable.
        assert_eq!(
            ResolvedLoadRequest {
                target_kind: LoadTargetKind::Extension,
                extension: Some("SalesAddon".to_owned()),
                ..configuration
            }
            .comparison_name(),
            Some("SalesAddon")
        );
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(unix)]
    fn write_designer_script(path: &Path, calls_log: &Path) {
        write_designer_script_with_merge_failure(path, calls_log, false);
    }

    #[cfg(unix)]
    fn write_designer_script_with_merge_failure(path: &Path, calls_log: &Path, fail_merge: bool) {
        let merge_block = if fail_merge {
            "if printf '%s' \"$*\" | grep -F -q -- '/MergeCfg'; then\n  printf 'merge failed\\n' >&2\n  exit 23\nfi\n"
        } else {
            ""
        };
        let body = format!(
            "args=\"$*\"\nout=\"\"\nreport=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  if [ \"$prev\" = \"-ReportFile\" ]; then report=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"{}\"\nif [ -n \"$out\" ]; then mkdir -p \"$(dirname \"$out\")\"; : > \"$out\"; fi\nif printf '%s' \"$args\" | grep -F -q -- '/CompareCfg'; then\n  if printf '%s' \"$args\" | grep -F -q -- 'VendorConfiguration'; then\n    printf 'Configuration Vendor configuration is not available\\n' > \"$out\"\n    exit 17\n  fi\n  if printf '%s' \"$args\" | grep -F -q -- 'ExtensionDBConfiguration'; then\n    if printf '%s' \"$args\" | grep -F -q -- 'ExistingExt'; then\n      : > \"$report\"\n      exit 0\n    fi\n    if printf '%s' \"$args\" | grep -F -q -- 'UnsupportedExt'; then\n      printf 'Configuration extension is not supported\\n' > \"$out\"\n      exit 18\n    fi\n    printf 'extension not found\\n' > \"$out\"\n    exit 19\n  fi\nfi\n{merge_block}exit 0",
            calls_log.display()
        );
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    fn sample_config(root: &Path, binary: &Path) -> AppConfig {
        AppConfig {
            base_path: root.to_path_buf(),
            work_path: root.join("work"),
            execution_timeout: 300_000,
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            source_sets: vec![],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(binary.to_path_buf()),
                    strict: false,
                    version: None,
                },
                enterprise: Default::default(),
                edt_cli: Default::default(),
                ..Default::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    /// A fake `ibcmd` that answers the one structural question the load path asks: which
    /// extensions the infobase lists. The records are written to a file and the script only
    /// `cat`s it — shell escaping differs between `bash` and `dash`, and an escaped quote that
    /// survives on one and not the other turned a listed extension into an absent one on Linux.
    #[cfg(unix)]
    fn write_extension_list_ibcmd(path: &Path, installed: &[&str]) {
        let records = installed
            .iter()
            .map(|name| {
                format!(
                    "name                         : \"{name}\"\nversion                      : \nactive                       : yes\npurpose                      : customization\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"{name}-hash\"\n\n"
                )
            })
            .collect::<String>();
        let records_path = path.with_extension("records");
        fs::write(&records_path, records).expect("write records");
        fs::write(
            path,
            format!("#!/bin/sh\ncat \"{}\"\nexit 0\n", records_path.display()),
        )
        .expect("write ibcmd");
        make_executable(path);
    }

    #[cfg(unix)]
    fn write_absent_extension_designer_script(
        path: &Path,
        calls_log: &Path,
        extra_stdout: Option<&str>,
        extra_stderr: Option<&str>,
        extra_log_line: Option<&str>,
    ) {
        let extra_stdout = extra_stdout.unwrap_or_default();
        let extra_stderr = extra_stderr.unwrap_or_default();
        let extra_log_line = extra_log_line.unwrap_or_default();
        let body = format!(
            "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"{}\"\nif printf '%s' \"$args\" | grep -F -q -- '/CompareCfg'; then\n  printf \"Конфигурация 'Расширение конфигурации' недоступна\\n{}\" > \"$out\"\n  printf '{}'\n  printf '{}' >&2\n  exit 19\nfi\nif [ -n \"$out\" ]; then : > \"$out\"; fi\nexit 0",
            calls_log.display(), extra_log_line, extra_stdout, extra_stderr
        );
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    /// https://github.com/IngvarConsulting/unica/issues/355
    #[cfg(unix)]
    #[test]
    fn execute_first_load_of_absent_extension_uses_distinct_state_and_applies_artifact() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        write_absent_extension_designer_script(&binary, &calls, None, None, None);
        write_extension_list_ibcmd(&root.join("ibcmd"), &[]);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: None,
            extension: Some("FirstExt".to_owned()),
        };

        let result = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect("first load should succeed");

        assert_eq!(
            load_payload(&result).compatibility_state,
            CompatibilityState::Absent
        );
        assert_eq!(
            serde_json::to_value(load_payload(&result)).expect("json")["compatibility_state"],
            "absent"
        );
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(
            !calls.contains("/CompareCfg"),
            "absence is established from the infobase's own list, so nothing is compared: \
             {calls}"
        );
        let ordered =
            ["/LoadCfg", "/UpdateDBCfg"].map(|needle| calls.find(needle).expect("expected call"));
        assert!(ordered[0] < ordered[1]);
    }

    /// Four tests used to stand here, each proving that a particular mix of stdout, stderr and
    /// `/Out` lines kept the state `unknown` and blocked the load. They classified the
    /// platform's prose, which ADR-0029 forbids, so the rule they protected is proven with a
    /// structural input instead: the infobase cannot be asked at all, and nothing is applied.
    #[cfg(unix)]
    #[test]
    fn an_extension_whose_presence_cannot_be_read_blocks_the_load() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        write_absent_extension_designer_script(&binary, &calls, None, None, None);
        // The infobase cannot be asked, so nothing about the extension is established.
        let ibcmd = root.join("ibcmd");
        fs::write(&ibcmd, "#!/bin/sh\nexit 7\n").expect("write ibcmd");
        make_executable(&ibcmd);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: None,
            extension: Some("ListedExt".to_owned()),
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("an unproven state must not permit a change");
        let payload = failure.payload.expect("payload");
        assert_eq!(
            load_payload(&payload).compatibility_state,
            CompatibilityState::NotEstablished
        );
        assert!(
            !calls.exists(),
            "nothing may be applied, or even started, on an unproven state"
        );
    }

    #[cfg(unix)]
    #[test]
    fn merge_of_absent_extension_returns_first_installation_hint() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        fs::write(root.join("merge.xml"), "<settings/>").expect("settings");
        write_absent_extension_designer_script(&binary, &calls, None, None, None);
        write_extension_list_ibcmd(&root.join("ibcmd"), &[]);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Merge,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: Some("merge.xml".to_owned()),
            extension: Some("FirstExt".to_owned()),
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("merge requires an installed extension");
        let payload = failure.payload.expect("payload");

        assert_eq!(
            load_payload(&payload).compatibility_state,
            CompatibilityState::Absent
        );
        assert_eq!(
            load_message(&payload),
            "validation error: extension 'FirstExt' is absent from the infobase; use --mode load for the first installation"
        );
        // The infobase answered the question, so Designer was never started at all.
        assert!(
            !calls.exists(),
            "a refusal established from the infobase list must dispatch no platform command"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_request_rejects_unsupported_artifacts_and_missing_flags() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("tool.epf"), "epf").expect("write");
        fs::write(root.join("ext.cfe"), "cfe").expect("write");
        let config = sample_config(root, &root.join("1cv8"));

        let unsupported = resolve_request(
            &config,
            &LoadRequest {
                vendor_name: None,
                dry_run: false,
                mode: LoadMode::Load,
                artifact_path: "tool.epf".to_owned(),
                settings_path: None,
                extension: None,
            },
        )
        .expect_err("epf should fail");
        assert!(unsupported.to_string().contains("only .cf and .cfe"));

        let missing_extension = resolve_request(
            &config,
            &LoadRequest {
                vendor_name: None,
                dry_run: false,
                mode: LoadMode::Load,
                artifact_path: "ext.cfe".to_owned(),
                settings_path: None,
                extension: None,
            },
        )
        .expect_err("cfe without extension should fail");
        assert!(missing_extension
            .to_string()
            .contains("require --extension"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_load_cf_loads_and_updates_without_asking() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("main.cf"), "cf").expect("artifact");
        write_designer_script(&binary, &calls);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "main.cf".to_owned(),
            settings_path: None,
            extension: None,
        };

        let result = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect("load result");

        assert!(result.execution.is_ok());
        assert_eq!(result.artifact_type, ArtifactBuildMode::ConfigurationCf);
        assert_eq!(
            load_payload(&result).compatibility_state,
            CompatibilityState::NotProbed
        );
        assert_eq!(load_payload(&result).update_db_cfg_ran, true);
        let calls_text = fs::read_to_string(calls).expect("calls");
        assert!(
            !calls_text.contains("/CompareCfg"),
            "without --vendor-name there is no question to ask, so nothing is compared: \
             {calls_text}"
        );
        assert!(calls_text.contains("/LoadCfg"));
        assert!(calls_text.contains("/UpdateDBCfg"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_unsupported_matrix_with_real_cfe_metadata() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        let mut config = sample_config(root, &root.join("1cv8"));
        // Валидация конфига такого ключа не пропустит: у `load` один исполнитель. Здесь
        // проверяется вторая линия — сценарий отказывает сам, если матрицу обошли.
        config.providers = [(
            crate::domain::capability::Operation::Load,
            crate::domain::capability::Provider::Ibcmd,
        )]
        .into();

        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: None,
            extension: Some("ExistingExt".to_owned()),
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("matrix should reject");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert!(!payload.execution.is_ok());
        assert_eq!(payload.artifact_type, ArtifactBuildMode::ExtensionCfe);
        assert_eq!(
            load_payload(&payload).target_kind,
            LoadTargetKind::Extension
        );
        assert_eq!(payload.extension.as_deref(), Some("ExistingExt"));
        assert!(load_message(&payload).contains("the Designer provider and format=DESIGNER"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_reports_cancelled_status_before_load_probe() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("main.cf"), "cf").expect("artifact");
        write_designer_script(&binary, &calls);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "main.cf".to_owned(),
            settings_path: None,
            extension: None,
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Load).with_cancellation(cancellation);

        let failure = execute(&context, &config, &request).expect_err("cancelled");
        let payload = failure.payload.expect("payload");

        assert_eq!(payload.execution.status, ExecutionStatus::Cancelled);
        assert_eq!(payload.execution.interruptions.len(), 1);
        assert!(payload.execution.errors[0]
            .message
            .contains("before entering load probe"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_merge_cfe_merges_and_updates_after_the_list() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        fs::write(root.join("merge.xml"), "<settings/>").expect("settings");
        write_designer_script(&binary, &calls);
        write_extension_list_ibcmd(&root.join("ibcmd"), &["ExistingExt"]);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Merge,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: Some("merge.xml".to_owned()),
            extension: Some("ExistingExt".to_owned()),
        };

        let result = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect("merge result");

        assert!(result.execution.is_ok());
        assert_eq!(
            load_payload(&result).compatibility_state,
            CompatibilityState::Supported
        );
        let calls_text = fs::read_to_string(calls).expect("calls");
        assert!(
            !calls_text.contains("/CompareCfg"),
            "an extension is answered by the infobase list, not by a comparison: {calls_text}"
        );
        assert!(calls_text.contains("/MergeCfg"));
        assert!(calls_text.contains("-Settings"));
        assert!(calls_text.contains("-Extension ExistingExt"));
        assert!(calls_text.contains("/UpdateDBCfg -Extension ExistingExt"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_load_cfe_with_unsupported_extension_runs_load_path() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        write_designer_script(&binary, &calls);
        write_extension_list_ibcmd(&root.join("ibcmd"), &["UnsupportedExt"]);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: None,
            extension: Some("UnsupportedExt".to_owned()),
        };

        let result = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect("load result");

        assert!(result.execution.is_ok());
        assert_eq!(
            load_payload(&result).compatibility_state,
            CompatibilityState::Supported
        );
        let calls_text = fs::read_to_string(calls).expect("calls");
        assert!(!calls_text.contains("/CompareCfg"), "{calls_text}");
        assert!(calls_text.contains("/LoadCfg"));
        assert!(calls_text.contains("/UpdateDBCfg -Extension UnsupportedExt"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_merge_cf_without_a_vendor_name_is_refused_before_the_platform_starts() {
        // The state that used to stand here — "extension is not supported" — was recognised by
        // a sentence the platform writes in no language, so it never occurred. This is the
        // refusal that does: a configuration merge cannot even be asked about without the
        // vendor configuration's name.
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("main.cf"), "cf").expect("artifact");
        fs::write(root.join("merge.xml"), "<settings/>").expect("settings");
        write_designer_script(&binary, &calls);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Merge,
            artifact_path: "main.cf".to_owned(),
            settings_path: Some("merge.xml".to_owned()),
            extension: None,
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("merge without a vendor name must be refused");
        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert_eq!(
            load_payload(&payload).compatibility_state,
            CompatibilityState::NotProbed
        );
        assert!(load_message(&payload).contains("--vendor-name"));
        assert!(
            !fs::read_to_string(&calls)
                .unwrap_or_default()
                .contains("/MergeCfg"),
            "nothing is merged when the question could not be asked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn execute_merge_failure_message_uses_merge_action_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        let binary = root.join("1cv8");
        let calls = root.join("calls.log");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        fs::write(root.join("merge.xml"), "<settings/>").expect("settings");
        write_designer_script(&binary, &calls);
        write_extension_list_ibcmd(&root.join("ibcmd"), &["ExistingExt"]);
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nif printf '%s' \"$*\" | grep -F -q -- '/MergeCfg'; then\n  echo 'merge failed' >&2\n  exit 23\nfi\n{}\n",
                fs::read_to_string(&binary).expect("script")
            ),
        )
        .expect("rewrite script");
        make_executable(&binary);
        let config = sample_config(root, &binary);
        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Merge,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: Some("merge.xml".to_owned()),
            extension: Some("ExistingExt".to_owned()),
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("merge should fail");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        let payload = failure.payload.expect("payload");
        assert!(!payload.execution.is_ok());
        assert_eq!(
            load_payload(&payload).compatibility_state,
            CompatibilityState::Supported
        );
        assert!(load_message(&payload).contains("merge failed for extension"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_cf_with_extension_payload_keeps_configuration_target_kind() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("main.cf"), "cf").expect("artifact");
        let config = sample_config(root, &root.join("1cv8"));

        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "main.cf".to_owned(),
            settings_path: None,
            extension: Some("Ignored".to_owned()),
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("cf with extension should fail");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.artifact_type, ArtifactBuildMode::ConfigurationCf);
        assert_eq!(
            load_payload(&payload).target_kind,
            LoadTargetKind::Configuration
        );
        assert_eq!(payload.extension.as_deref(), Some("Ignored"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_cfe_without_extension_payload_keeps_extension_target_kind() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("work")).expect("work");
        fs::write(root.join("ext.cfe"), "cfe").expect("artifact");
        let config = sample_config(root, &root.join("1cv8"));

        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "ext.cfe".to_owned(),
            settings_path: None,
            extension: None,
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("cfe without extension should fail");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.artifact_type, ArtifactBuildMode::ExtensionCfe);
        assert_eq!(
            load_payload(&payload).target_kind,
            LoadTargetKind::Extension
        );
        assert_eq!(payload.extension.as_deref(), None);
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_unknown_artifact_payload_marks_unknown_metadata() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("release.zip"), "zip").expect("artifact");
        let config = sample_config(root, &root.join("1cv8"));

        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "release.zip".to_owned(),
            settings_path: None,
            extension: None,
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("zip should fail");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.artifact_type, ArtifactBuildMode::Unknown);
        assert_eq!(load_payload(&payload).target_kind, LoadTargetKind::Unknown);
        assert_eq!(payload.extension.as_deref(), None);
        assert!(load_message(&payload).contains("only .cf and .cfe"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_external_artifact_payload_marks_unknown_target_kind() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("tool.epf"), "epf").expect("artifact");
        let config = sample_config(root, &root.join("1cv8"));

        let request = LoadRequest {
            vendor_name: None,
            dry_run: false,
            mode: LoadMode::Load,
            artifact_path: "tool.epf".to_owned(),
            settings_path: None,
            extension: None,
        };

        let failure = execute(&ExecutionContext::cli(CommandName::Load), &config, &request)
            .expect_err("epf should fail");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        let payload = failure.payload.expect("payload");
        assert_eq!(
            payload.artifact_type,
            ArtifactBuildMode::ExternalDataProcessorEpf
        );
        assert_eq!(load_payload(&payload).target_kind, LoadTargetKind::Unknown);
        assert_eq!(payload.extension.as_deref(), None);
        assert!(load_message(&payload).contains("only .cf and .cfe"));
    }
}
