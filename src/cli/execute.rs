use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::cli::args::{
    ArtifactsArgs, BuildArgs, Command, ConvertArgs, DesignerConfigSyntaxArgs,
    DesignerModulesSyntaxArgs, DirectLaunchOptionsArgs, DumpArgs, ExtensionsArgs,
    ExtensionsCommand, InfobaseArgs, InfobaseCommand, InfobaseConfigurationCommand,
    InfobaseConfigurationExportArgs, InfobaseRestoreArgs, LaunchArgs, LaunchOptionsArgs, LoadArgs,
    SyntaxArgs, SyntaxTarget, TestArgs, TestLaunchOptionsArgs, TestRunner, TestScope, TestVaArgs,
    TestYaxunitArgs, ToolsArgs, ToolsCommand, ToolsDownloadArgs, ToolsDownloadCommand,
};
use crate::cli::output::{
    failure_envelope, pre_dispatch_error_envelope, print_command_use_case_error, with_cli_error,
};
use crate::cli::signal::CliSignalGuard;
use crate::command_envelope::{test_envelope, Envelope};
use crate::config::model::{AppConfig, SourceFormat, SourceSetPurpose};
use crate::domain::artifact::{
    ArtifactRef, ArtifactSet, ARTIFACT_ROLE_PACKAGE_FILE, ARTIFACT_ROLE_PLATFORM_LOG,
};
use crate::domain::artifacts::{ArtifactBuildMetadata, ArtifactBuildMode, ArtifactsResult};
use crate::domain::build::{BuildMode, BuildResult};
use crate::domain::capability::ProviderReceipt;
use crate::domain::convert::{ConvertDirection, ConvertResult, ConvertScope};
use crate::domain::dump::{DumpMode, DumpResult};
use crate::domain::execution::{
    ExecutionError, ExecutionInterruptionDetails, ExecutionOutcome, ExecutionStatus,
    ExecutionStepStatus, StepResult,
};
use crate::domain::infobase_export::{
    ConfigurationState, ConfigurationSubject, ExportConfigurationPackageRequest,
    ExportConfigurationPackageResult, ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult,
    InfobaseTransferPhase, RestoreInfobaseSnapshotRequest, RestoreInfobaseSnapshotResult,
    RestoreTargetMode,
};
use crate::domain::init::{InitResult, InitStep, InitStepStatus};
use crate::domain::issue::{Issue, IssueSeverity};
use crate::domain::launch::{LaunchMode, LaunchResult, LaunchVia};
use crate::domain::load::{
    CompatibilityState, LoadExecutionMetadata, LoadMode, LoadResult, LoadTargetKind,
};
use crate::domain::runner::{
    launch_key_alias_matches, ExecutionPolicy, ExternalEpfWaitOptions, LaunchClientModeRequest,
    LaunchOptions, RunnerKind, RunnerOutputFormat, RunnerProfile,
};
use crate::domain::syntax::{SyntaxCheckResult, SyntaxCheckStatus};
use crate::domain::test::{RetainedPaths, TestReport, TestRunResult, TestStatus, TestTarget};
use crate::domain::tools_download::{
    ToolDownloadTarget, ToolExtensionInstallMode, ToolsDownloadResult,
};
use crate::output::presenter::Presenter;
use crate::output::text::{TimelineItem, TimelineStatus};
use crate::support::adapter_input::{
    parse_launch_target, parse_required_dump_mode, LaunchModeAliases,
};
use crate::support::error::AppError;
use crate::support::fs::clean_dir;
use crate::support::path::is_safe_path_segment;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::artifacts;
use crate::use_cases::build_project;
use crate::use_cases::check_syntax;
use crate::use_cases::configure_extensions;
use crate::use_cases::context::{CommandName, ExecutionContext};
use crate::use_cases::convert_sources;
use crate::use_cases::dump_config;
use crate::use_cases::extension_inventory;
use crate::use_cases::extension_inventory::ExtensionChangeRequest;
use crate::use_cases::infobase_export;
use crate::use_cases::init_project;
use crate::use_cases::launch_app;
use crate::use_cases::load_artifact;
use crate::use_cases::request::{
    effective_test_timeouts, ArtifactsModeRequest, ArtifactsRequest, BuildRequest,
    ClientMcpAddonRequest, ClientMcpMode, ClientMcpOptionsRequest, ConfigureExtensionsRequest,
    ConvertRequest, ConvertScopeRequest, DesignerClientScope, DesignerClientScopes,
    DesignerConfigCheck, DesignerConfigChecks, DesignerConfigSyntaxRequest, DumpRequest,
    ExtensionInventoryRequest, ExtensionInventoryScope, InitRequest, LaunchRequest, LoadRequest,
    SyntaxExtensionScope, SyntaxRequest, SyntaxTargetRequest, TestRequest, TestScopeRequest,
    ToolsDownloadRequest,
};
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};
use crate::use_cases::run_tests;
use crate::use_cases::tools_download;
use crate::use_cases::transport::{dispatch_with_workspace_lock_policy, WorkspaceBusyPolicy};

/// Executes a parsed CLI command by mapping it into transport-neutral requests and
/// rendering the resulting command output.
pub fn execute_command(
    config: &AppConfig,
    command: &Command,
    primary_config_path: Option<PathBuf>,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
) -> Result<(), UseCaseError> {
    let cancellation = CancellationToken::new();
    let _signal_guard = CliSignalGuard::install(cancellation.clone());
    match command {
        Command::Version => unreachable!("version command is handled outside cli::execute"),
        Command::Bootstrap(_) => unreachable!("bootstrap command is handled outside cli::execute"),
        Command::Config(_) | Command::ConfigInit(_) => {
            unreachable!("config commands are handled outside cli::execute")
        }
        Command::Download(_) => unreachable!("download is normalised into infobase in app::run"),
        Command::Tools(args) => execute_tools(
            config,
            args,
            required_primary_config_path(primary_config_path)?,
            presenter,
            clean_before_execution,
            cancellation,
        ),
        Command::Extensions(args) => execute_extensions(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Build(args) => execute_build(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Load(args) => execute_load(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Test(args) => execute_test(
            config,
            args,
            presenter,
            clean_before_execution,
            cancellation,
        ),
        Command::Dump(args) => execute_dump(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Init => execute_init(
            config,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Infobase(args) => execute_infobase(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Convert(args) => execute_convert(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Artifacts(args) => execute_artifacts(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Syntax(args) => execute_syntax(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Launch(args) => execute_launch(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Publish(args) => execute_publish(
            config,
            args,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        ),
        Command::Mcp(_) => unreachable!("mcp commands are handled outside cli::execute"),
    }
}

fn execute_publish(
    config: &AppConfig,
    args: &crate::cli::args::PublishArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    use crate::domain::publish::PublishAction;
    use crate::use_cases::publish_infobase::{self, PublishRequest};

    let request = PublishRequest {
        action: if args.delete {
            PublishAction::Delete
        } else {
            PublishAction::Publish
        },
        dry_run,
    };
    let context = cli_context(config, CommandName::Publish, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Publish,
        clean_before_execution,
        dry_run,
        || match publish_infobase::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Publish.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_publish_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    match failure.payload {
                        Some(result) => presenter.print_envelope(&failure_envelope(
                            CommandName::Publish.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        )),
                        None => presenter.print_envelope(&pre_dispatch_error_envelope(
                            CommandName::Publish.as_str(),
                            &error,
                        )),
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_publish_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn render_publish_text(
    result: &crate::domain::publish::PublishResult,
    presenter: &Presenter,
    succeeded: bool,
) {
    let verb = match result.action {
        crate::domain::publish::PublishAction::Publish => "Publication",
        crate::domain::publish::PublishAction::Delete => "Publication removal",
    };
    // «Запланировано» — не стандартный исход, у него своя подпись.
    let planned_label =
        (succeeded && !result.provider_dispatched).then(|| format!("{verb} planned"));
    let mut details = vec![
        format!("server: {}", result.server),
        format!("wsdir: {}", result.wsdir),
        format!("dir: {}", result.dir.display()),
    ];
    if let Some(url) = result.url.as_deref() {
        details.push(format!("url: {url}"));
    }
    if !result.provider_dispatched {
        details.push("provider dispatched: false".to_owned());
    }
    if let Some(plan) = &result.plan {
        details.push(format!("planned program: {}", plan.program.display()));
        details.push(format!("planned args: {}", plan.args.join(" ")));
    }
    append_if_present(
        &mut details,
        result
            .message
            .as_deref()
            .map(|message| bracketed_detail(if succeeded { "status" } else { "error" }, message)),
    );
    append_if_present(
        &mut details,
        result
            .platform_log_path
            .as_deref()
            .map(|path| format!("[diagnostic] platform log -> {}", path.display())),
    );
    details.extend(provider_receipt_details(result.provider.as_ref()));
    match planned_label {
        Some(label) => single_timeline(presenter, timeline_status(succeeded), label, details),
        None => single_timeline_outcome(presenter, timeline_status(succeeded), verb, details),
    }
}

/// Returns the canonical command identifier for a parsed CLI command.
pub fn command_name(command: &Command) -> CommandName {
    match command {
        Command::Version => unreachable!("version command does not map to execution use cases"),
        Command::Bootstrap(_) => CommandName::Bootstrap,
        Command::Config(_) | Command::ConfigInit(_) => {
            unreachable!("config commands do not map to execution use cases")
        }
        Command::Download(_) => unreachable!("download is normalised into infobase in app::run"),
        Command::Tools(ToolsArgs {
            command: ToolsCommand::Download(_),
        }) => CommandName::ToolsDownload,
        Command::Extensions(_) => CommandName::Extensions,
        Command::Build(_) => CommandName::Build,
        Command::Load(_) => CommandName::Load,
        Command::Test(_) => CommandName::Test,
        Command::Dump(_) => CommandName::Dump,
        Command::Infobase(InfobaseArgs {
            command:
                InfobaseCommand::Configuration(crate::cli::args::InfobaseConfigurationArgs {
                    command: InfobaseConfigurationCommand::Export(_),
                }),
        }) => CommandName::InfobaseConfigurationExport,
        Command::Init => CommandName::Init,
        Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Create,
        }) => unreachable!("infobase create is normalised into its own command in app::run"),
        Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Dump(_),
        }) => CommandName::InfobaseDump,
        Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Restore(_),
        }) => CommandName::InfobaseRestore,
        Command::Convert(_) => CommandName::Convert,
        Command::Artifacts(_) => CommandName::Artifacts,
        Command::Syntax(_) => CommandName::Syntax,
        Command::Launch(_) => CommandName::Launch,
        Command::Publish(_) => CommandName::Publish,
        Command::Mcp(_) => unreachable!("mcp commands do not map to CLI command names"),
    }
}

pub fn uses_infobase_export_config(command: &Command) -> bool {
    matches!(
        command,
        Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Configuration(crate::cli::args::InfobaseConfigurationArgs {
                command: InfobaseConfigurationCommand::Export(_),
            }) | InfobaseCommand::Dump(_)
                | InfobaseCommand::Restore(_),
        })
    )
}

fn execute_tools(
    config: &AppConfig,
    args: &ToolsArgs,
    primary_config_path: PathBuf,
    presenter: &Presenter,
    clean_before_execution: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    match &args.command {
        ToolsCommand::Download(download) => execute_tools_download(
            config,
            download,
            primary_config_path,
            presenter,
            clean_before_execution,
            cancellation,
        ),
    }
}

fn execute_tools_download(
    config: &AppConfig,
    args: &ToolsDownloadArgs,
    primary_config_path: PathBuf,
    presenter: &Presenter,
    clean_before_execution: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = ToolsDownloadRequest {
        config_path: primary_config_path,
        target: map_tools_download_target(args),
        extensions: map_tool_extension_mode(args),
        force: map_tools_download_force(args),
    };
    let context = cli_context(config, CommandName::ToolsDownload, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::ToolsDownload,
        clean_before_execution,
        // превью у загрузки инструментов нет.
        false,
        || match tools_download::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::ToolsDownload.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_tools_download_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    match failure.payload {
                        Some(result) => presenter.print_envelope(&failure_envelope(
                            CommandName::ToolsDownload.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        )),
                        None => presenter.print_envelope(&pre_dispatch_error_envelope(
                            CommandName::ToolsDownload.as_str(),
                            &error,
                        )),
                    }
                } else {
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn required_primary_config_path(
    primary_config_path: Option<PathBuf>,
) -> Result<PathBuf, UseCaseError> {
    primary_config_path.ok_or_else(|| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            "tools download requires a resolved primary config path",
        )
    })
}

fn execute_extensions(
    config: &AppConfig,
    args: &ExtensionsArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    args.validate_property_options().map_err(|message| {
        render_pre_dispatch_error(
            presenter,
            CommandName::Extensions,
            AppError::Validation(message.to_owned()),
        )
    })?;
    if let Some(command) = &args.command {
        return execute_extension_command(
            config,
            command,
            presenter,
            clean_before_execution,
            dry_run,
            cancellation,
        );
    }
    let request = map_extensions_request(args, dry_run);
    configure_extensions::resolve_targets(config, &request)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Extensions, error))?;
    let context = cli_context(config, CommandName::Extensions, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Extensions,
        clean_before_execution,
        dry_run,
        || match configure_extensions::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Extensions.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else if !result.provider_dispatched {
                    render_extensions_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    match failure.payload {
                        Some(result) => presenter.print_envelope(&failure_envelope(
                            CommandName::Extensions.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        )),
                        None => presenter.print_envelope(&pre_dispatch_error_envelope(
                            CommandName::Extensions.as_str(),
                            &error,
                        )),
                    }
                } else {
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

/// Dispatches the infobase-side extension family.
///
/// Reads and writes share one workspace lock boundary: the composition can change under
/// a read, and a listing taken across an install would report a half-state.
fn execute_extension_command(
    config: &AppConfig,
    command: &ExtensionsCommand,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let context = cli_context(config, CommandName::Extensions, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Extensions,
        clean_before_execution,
        // Превью любой подкоманды платформу не поднимает, значит и `workPath` ему не нужен.
        dry_run,
        || match command {
            ExtensionsCommand::List => run_extension_inventory(
                config,
                &context,
                presenter,
                ExtensionInventoryScope::All,
                dry_run,
            ),
            ExtensionsCommand::Info(args) => run_extension_inventory(
                config,
                &context,
                presenter,
                ExtensionInventoryScope::Named {
                    name: args.name.clone(),
                },
                dry_run,
            ),
            ExtensionsCommand::Create(args) => run_extension_change(
                config,
                &context,
                presenter,
                ExtensionChangeRequest::Create {
                    name: args.name.clone(),
                    name_prefix: args.name_prefix.clone(),
                    synonym: args.synonym.clone(),
                    purpose: args.purpose.clone(),
                },
                dry_run,
            ),
            ExtensionsCommand::Delete(args) => run_extension_change(
                config,
                &context,
                presenter,
                ExtensionChangeRequest::Delete {
                    name: args.name.clone(),
                },
                dry_run,
            ),
            ExtensionsCommand::Activate(args) => run_extension_change(
                config,
                &context,
                presenter,
                ExtensionChangeRequest::SetActive {
                    name: args.name.clone(),
                    active: args.active == "yes",
                },
                dry_run,
            ),
        },
    )
}

fn run_extension_inventory(
    config: &AppConfig,
    context: &ExecutionContext,
    presenter: &Presenter,
    scope: ExtensionInventoryScope,
    dry_run: bool,
) -> Result<(), UseCaseError> {
    let request = ExtensionInventoryRequest { scope, dry_run };
    match extension_inventory::execute(context, config, &request) {
        Ok(result) => {
            if presenter.is_json() {
                presenter.print_envelope(&Envelope::ok(
                    CommandName::Extensions.as_str(),
                    result.duration_ms,
                    result,
                ));
            } else {
                render_extension_inventory_text(&result, presenter);
            }
            Ok(())
        }
        Err(failure) => {
            let error = failure.error;
            if presenter.is_json() {
                presenter.print_envelope(&pre_dispatch_error_envelope(
                    CommandName::Extensions.as_str(),
                    &error,
                ));
            } else {
                presenter.print_error(&error.to_string());
            }
            Err(error)
        }
    }
}

fn run_extension_change(
    config: &AppConfig,
    context: &ExecutionContext,
    presenter: &Presenter,
    request: ExtensionChangeRequest,
    dry_run: bool,
) -> Result<(), UseCaseError> {
    match extension_inventory::change(context, config, &request, dry_run) {
        Ok(result) => {
            if presenter.is_json() {
                presenter.print_envelope(&Envelope::ok(
                    CommandName::Extensions.as_str(),
                    result.duration_ms,
                    result,
                ));
            } else {
                render_extensions_text(&result, presenter);
            }
            Ok(())
        }
        Err(failure) => {
            let error = failure.error;
            if presenter.is_json() {
                match failure.payload {
                    Some(result) => presenter.print_envelope(&failure_envelope(
                        CommandName::Extensions.as_str(),
                        result.duration_ms,
                        result,
                        &error,
                    )),
                    None => presenter.print_envelope(&pre_dispatch_error_envelope(
                        CommandName::Extensions.as_str(),
                        &error,
                    )),
                }
            } else {
                presenter.print_error(&error.to_string());
            }
            Err(error)
        }
    }
}

fn render_extension_inventory_text(
    result: &crate::domain::extensions::ExtensionInventoryResult,
    presenter: &Presenter,
) {
    if let Some(plan) = result.plan.as_deref() {
        let requested = match &result.requested {
            crate::domain::extensions::RequestedInventory::All => {
                "requested: every installed extension".to_owned()
            }
            crate::domain::extensions::RequestedInventory::Named { name } => {
                format!("requested: extension '{name}'")
            }
        };
        presenter.print_timeline(&[TimelineItem::new(
            TimelineStatus::Succeeded,
            "Infobase extensions preview",
        )
        .with_detail(format!("{requested}\n{plan}"))]);
        return;
    }
    if result.extensions.is_empty() {
        presenter.print_timeline(&[TimelineItem::new(
            TimelineStatus::Succeeded,
            "Infobase extensions",
        )
        .with_detail("no extensions are installed in the infobase".to_owned())]);
        return;
    }
    let details = result
        .extensions
        .iter()
        .map(|extension| {
            format!(
                "{}: purpose={}, active={}, safe mode={}, unsafe action protection={}, scope={}, version={}, prefix={}, hash={}",
                extension.name,
                extension.purpose,
                extension.active,
                extension.safe_mode,
                extension.unsafe_action_protection,
                extension.scope,
                extension.version.as_deref().unwrap_or("none"),
                extension.name_prefix.as_deref().unwrap_or("unavailable"),
                extension.hash_sum,
            )
        })
        .collect::<Vec<_>>();
    let mut details = details;
    details.extend(provider_receipt_details(result.provider.as_ref()));
    presenter.print_timeline(&[TimelineItem::new(
        TimelineStatus::Succeeded,
        "Infobase extensions",
    )
    .with_detail(details.join("\n"))]);
}

fn render_extensions_text(
    result: &crate::domain::extensions::ExtensionsResult,
    presenter: &Presenter,
) {
    let details = result
        .steps
        .iter()
        .map(|step| {
            format!(
                "{}: {} -> {}{}",
                step.target,
                step.action,
                // A preview performed nothing, so the step must not read as done.
                match (result.provider_dispatched, step.ok) {
                    (false, _) => "planned",
                    (true, true) => "ok",
                    (true, false) => "failed",
                },
                step.message
                    .as_deref()
                    .map(|message| format!(" ({message})"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    let status = if result.ok {
        TimelineStatus::Succeeded
    } else {
        TimelineStatus::Failed
    };
    let label = if result.provider_dispatched {
        "Infobase extension change"
    } else {
        "Infobase extension change preview"
    };
    let mut details = details;
    details.extend(provider_receipt_details(result.provider.as_ref()));
    presenter.print_timeline(&[TimelineItem::new(status, label).with_detail(details.join("\n"))]);
}

fn execute_init(
    config: &AppConfig,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = InitRequest { dry_run };
    let context = cli_context(config, CommandName::Init, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Init,
        clean_before_execution,
        dry_run,
        || match init_project::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Init.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_init_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        presenter.print_envelope(&failure_envelope(
                            CommandName::Init.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        ));
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_init_text(result, presenter);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_build(
    config: &AppConfig,
    args: &BuildArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_build_request(args, dry_run);
    let context = cli_context(config, CommandName::Build, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Build,
        clean_before_execution,
        dry_run,
        || match build_project::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Build.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_build_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        presenter.print_envelope(&failure_envelope(
                            CommandName::Build.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        ));
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_build_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_test(
    config: &AppConfig,
    args: &TestArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_test_request(config, args)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Test, error))?;
    let effective_config = effective_test_config(config, args)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Test, error))?;
    let context = cli_context(&effective_config, CommandName::Test, cancellation);
    with_cli_workspace_lock(
        &effective_config,
        presenter,
        CommandName::Test,
        clean_before_execution,
        // превью у прогона тестов нет.
        false,
        || match run_tests::execute(&context, &effective_config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    let envelope = test_envelope(&result);
                    presenter.print_envelope(&envelope);
                } else {
                    render_test_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        let envelope = with_cli_error(test_envelope(&result), &error);
                        presenter.print_envelope(&envelope);
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_test_text(result, presenter);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_load(
    config: &AppConfig,
    args: &LoadArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_load_request(args, dry_run)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Load, error))?;
    let context = cli_context(config, CommandName::Load, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Load,
        clean_before_execution,
        dry_run,
        || match load_artifact::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    let envelope = build_load_envelope(&result);
                    presenter.print_envelope(&envelope);
                } else {
                    render_load_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        let envelope = with_cli_error(build_load_envelope(&result), &error);
                        presenter.print_envelope(&envelope);
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_load_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_dump(
    config: &AppConfig,
    args: &DumpArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_dump_request(args, dry_run)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Dump, error))?;
    let context = cli_context(config, CommandName::Dump, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Dump,
        clean_before_execution,
        dry_run,
        || match dump_config::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Dump.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_dump_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        presenter.print_envelope(&failure_envelope(
                            CommandName::Dump.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        ));
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_dump_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

pub enum PreparedInfobaseCommand {
    Configuration {
        request: ExportConfigurationPackageRequest,
        provider: infobase_export::PreparedTransferProvider,
    },
    Snapshot {
        request: ExportInfobaseSnapshotRequest,
        provider: infobase_export::PreparedTransferProvider,
    },
    Restore {
        request: RestoreInfobaseSnapshotRequest,
        provider: infobase_export::PreparedTransferProvider,
    },
}

pub struct PreparedInfobaseCliCommand {
    command: PreparedInfobaseCommand,
    context: ExecutionContext,
    _signal_guard: CliSignalGuard,
}

pub fn validate_infobase_request(args: &InfobaseArgs) -> Result<(), AppError> {
    match &args.command {
        InfobaseCommand::Create => {
            unreachable!("infobase create is normalised into its own command in app::run")
        }
        InfobaseCommand::Configuration(configuration) => match &configuration.command {
            InfobaseConfigurationCommand::Export(args) => {
                let request = map_infobase_configuration_export_request(args);
                infobase_export::validate_configuration_request(&request)
            }
        },
        InfobaseCommand::Dump(args) => {
            infobase_export::validate_snapshot_output(Path::new(&args.output))
        }
        InfobaseCommand::Restore(args) => {
            infobase_export::validate_restore_request(&map_infobase_restore_request(args)?)
        }
    }
}

/// Maps restore CLI arguments into the transport-neutral request.
///
/// Exactly one target mode must be stated: neither provider asks before creating or
/// overwriting an infobase, so the runner refuses to guess which one was meant.
fn map_infobase_restore_request(
    args: &InfobaseRestoreArgs,
) -> Result<RestoreInfobaseSnapshotRequest, AppError> {
    let target_mode = match (args.create, args.replace) {
        (true, false) => RestoreTargetMode::Create,
        (false, true) => RestoreTargetMode::Replace,
        _ => {
            return Err(AppError::Validation(
                "infobase restore requires exactly one of --create or --replace".to_owned(),
            ))
        }
    };
    Ok(RestoreInfobaseSnapshotRequest {
        input: PathBuf::from(&args.input),
        target_mode,
    })
}

pub fn render_invalid_infobase_request(
    args: &InfobaseArgs,
    presenter: &Presenter,
    error: AppError,
    dry_run: bool,
) -> UseCaseError {
    let error = UseCaseError::from(error);
    render_infobase_pre_dispatch_failure(
        args,
        presenter,
        error,
        "provider selection was not attempted because the request is invalid",
        InfobaseTransferPhase::Validation,
        dry_run,
    )
}

pub fn render_infobase_pre_dispatch_failure(
    args: &InfobaseArgs,
    presenter: &Presenter,
    error: UseCaseError,
    _selection_reason: &str,
    phase: InfobaseTransferPhase,
    dry_run: bool,
) -> UseCaseError {
    // Выбор исполнителя не начинался: квитанции нет, причина — в ошибке конверта.
    let selection: Option<ProviderReceipt> = None;
    match &args.command {
        InfobaseCommand::Create => {
            unreachable!("infobase create has no export request to render")
        }
        InfobaseCommand::Configuration(configuration) => match &configuration.command {
            InfobaseConfigurationCommand::Export(args) => {
                let request = map_infobase_configuration_export_request(args);
                let mut result =
                    configuration_pre_dispatch_failure(&request, selection.clone(), &error, phase);
                if dry_run {
                    result.mark_preview_failure();
                }
                render_configuration_failure(
                    CommandName::InfobaseConfigurationExport,
                    result,
                    &error,
                    presenter,
                );
            }
        },
        InfobaseCommand::Dump(args) => {
            let request = ExportInfobaseSnapshotRequest {
                output: PathBuf::from(&args.output),
            };
            let mut result =
                snapshot_pre_dispatch_failure(&request, selection.clone(), &error, phase);
            if dry_run {
                result.mark_preview_failure();
            }
            render_snapshot_failure(CommandName::InfobaseDump, result, &error, presenter);
        }
        InfobaseCommand::Restore(args) => {
            let request = map_infobase_restore_request(args).unwrap_or_else(|_| {
                // The mode is unreadable, so the rendered subject states the requested
                // input and leaves the mode to the error text.
                RestoreInfobaseSnapshotRequest {
                    input: PathBuf::from(&args.input),
                    target_mode: RestoreTargetMode::Replace,
                }
            });
            let mut result = restore_pre_dispatch_failure(&request, selection, &error, phase);
            if dry_run {
                result.mark_preview_failure();
            }
            render_restore_failure(CommandName::InfobaseRestore, result, &error, presenter);
        }
    }
    error
}

pub fn prepare_infobase_command(
    config: &AppConfig,
    args: &InfobaseArgs,
    presenter: &Presenter,
    context: &ExecutionContext,
    dry_run: bool,
) -> Result<PreparedInfobaseCommand, UseCaseError> {
    match &args.command {
        InfobaseCommand::Create => {
            unreachable!("infobase create is dispatched before the export machinery")
        }
        InfobaseCommand::Configuration(configuration) => match &configuration.command {
            InfobaseConfigurationCommand::Export(args) => {
                let request = map_infobase_configuration_export_request(args);
                let command = CommandName::InfobaseConfigurationExport;
                infobase_export::validate_configuration_request(&request)
                    .map_err(|error| render_pre_dispatch_error(presenter, command, error))?;
                match infobase_export::prepare_configuration_export(context, config, &request) {
                    Ok(provider) => {
                        Ok(PreparedInfobaseCommand::Configuration { request, provider })
                    }
                    Err(failure) => {
                        let error = failure.error;
                        if let Some(mut result) = failure.payload {
                            if dry_run {
                                result.mark_preview_failure();
                            }
                            render_configuration_failure(command, result, &error, presenter);
                        }
                        Err(error)
                    }
                }
            }
        },
        InfobaseCommand::Dump(args) => {
            let request = ExportInfobaseSnapshotRequest {
                output: PathBuf::from(&args.output),
            };
            let command = CommandName::InfobaseDump;
            infobase_export::validate_snapshot_output(&request.output)
                .map_err(|error| render_pre_dispatch_error(presenter, command, error))?;
            match infobase_export::prepare_infobase_snapshot(context, config, &request) {
                Ok(provider) => Ok(PreparedInfobaseCommand::Snapshot { request, provider }),
                Err(failure) => {
                    let error = failure.error;
                    if let Some(mut result) = failure.payload {
                        if dry_run {
                            result.mark_preview_failure();
                        }
                        render_snapshot_failure(command, result, &error, presenter);
                    }
                    Err(error)
                }
            }
        }
        InfobaseCommand::Restore(args) => {
            let command = CommandName::InfobaseRestore;
            let request = map_infobase_restore_request(args)
                .map_err(|error| render_pre_dispatch_error(presenter, command, error))?;
            infobase_export::validate_restore_request(&request)
                .map_err(|error| render_pre_dispatch_error(presenter, command, error))?;
            match infobase_export::prepare_infobase_restore(context, config, &request) {
                Ok(provider) => Ok(PreparedInfobaseCommand::Restore { request, provider }),
                Err(failure) => {
                    let error = failure.error;
                    if let Some(mut result) = failure.payload {
                        if dry_run {
                            result.mark_preview_failure();
                        }
                        render_restore_failure(command, result, &error, presenter);
                    }
                    Err(error)
                }
            }
        }
    }
}

pub fn prepare_infobase_cli_command(
    config: &AppConfig,
    args: &InfobaseArgs,
    presenter: &Presenter,
    dry_run: bool,
) -> Result<PreparedInfobaseCliCommand, UseCaseError> {
    let cancellation = CancellationToken::new();
    let signal_guard = CliSignalGuard::install(cancellation.clone());
    let context = cli_context(config, infobase_command_name(args), cancellation);
    let command = prepare_infobase_command(config, args, presenter, &context, dry_run)?;
    Ok(PreparedInfobaseCliCommand {
        command,
        context,
        _signal_guard: signal_guard,
    })
}

fn infobase_command_name(args: &InfobaseArgs) -> CommandName {
    match &args.command {
        InfobaseCommand::Create => {
            unreachable!("infobase create is normalised into its own command in app::run")
        }
        InfobaseCommand::Configuration(_) => CommandName::InfobaseConfigurationExport,
        InfobaseCommand::Dump(_) => CommandName::InfobaseDump,
        InfobaseCommand::Restore(_) => CommandName::InfobaseRestore,
    }
}

fn map_infobase_configuration_export_request(
    args: &InfobaseConfigurationExportArgs,
) -> ExportConfigurationPackageRequest {
    let state = match args.state.as_str() {
        "working" => ConfigurationState::Working,
        "database" => ConfigurationState::Database,
        _ => unreachable!("clap validates configuration state"),
    };
    let subject = args
        .extension
        .as_ref()
        .map(|name| ConfigurationSubject::Extension { name: name.clone() })
        .unwrap_or(ConfigurationSubject::Main);
    ExportConfigurationPackageRequest {
        state,
        subject,
        output: PathBuf::from(&args.output),
    }
}

fn execute_infobase(
    config: &AppConfig,
    args: &InfobaseArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let context = cli_context(config, infobase_command_name(args), cancellation);
    let prepared = prepare_infobase_command(config, args, presenter, &context, dry_run)?;
    execute_prepared_infobase(
        config,
        prepared,
        &context,
        presenter,
        clean_before_execution,
    )
}

pub fn execute_prepared_infobase(
    config: &AppConfig,
    prepared: PreparedInfobaseCommand,
    context: &ExecutionContext,
    presenter: &Presenter,
    clean_before_execution: bool,
) -> Result<(), UseCaseError> {
    match prepared {
        PreparedInfobaseCommand::Configuration { request, provider } => {
            execute_infobase_configuration_export(
                config,
                request,
                provider,
                context,
                presenter,
                clean_before_execution,
            )
        }
        PreparedInfobaseCommand::Snapshot { request, provider } => execute_infobase_dump(
            config,
            request,
            provider,
            context,
            presenter,
            clean_before_execution,
        ),
        PreparedInfobaseCommand::Restore { request, provider } => execute_infobase_restore(
            config,
            request,
            provider,
            context,
            presenter,
            clean_before_execution,
        ),
    }
}

pub fn execute_prepared_infobase_command(
    config: &AppConfig,
    prepared: PreparedInfobaseCliCommand,
    presenter: &Presenter,
    clean_before_execution: bool,
) -> Result<(), UseCaseError> {
    execute_prepared_infobase(
        config,
        prepared.command,
        &prepared.context,
        presenter,
        clean_before_execution,
    )
}

pub fn preview_prepared_infobase_command(
    config: &AppConfig,
    prepared: PreparedInfobaseCliCommand,
    presenter: &Presenter,
) -> Result<(), UseCaseError> {
    let context = prepared.context;
    match prepared.command {
        PreparedInfobaseCommand::Configuration { request, provider } => {
            match infobase_export::preview_configuration_export(
                &context, config, &request, &provider,
            ) {
                Ok(result) => {
                    if presenter.is_json() {
                        presenter.print_envelope(&Envelope::ok(
                            CommandName::InfobaseConfigurationExport.as_str(),
                            0,
                            result,
                        ));
                    } else {
                        render_configuration_export_text(
                            CommandName::InfobaseConfigurationExport,
                            &result,
                            presenter,
                        );
                    }
                }
                Err(failure) => {
                    let error = failure.error;
                    if let Some(result) = failure.payload {
                        render_configuration_failure(
                            CommandName::InfobaseConfigurationExport,
                            result,
                            &error,
                            presenter,
                        );
                    }
                    return Err(error);
                }
            }
        }
        PreparedInfobaseCommand::Snapshot { request, provider } => {
            match infobase_export::preview_infobase_snapshot(&context, config, &request, &provider)
            {
                Ok(result) => {
                    if presenter.is_json() {
                        presenter.print_envelope(&Envelope::ok(
                            CommandName::InfobaseDump.as_str(),
                            0,
                            result,
                        ));
                    } else {
                        render_snapshot_export_text(CommandName::InfobaseDump, &result, presenter);
                    }
                }
                Err(failure) => {
                    let error = failure.error;
                    if let Some(result) = failure.payload {
                        render_snapshot_failure(
                            CommandName::InfobaseDump,
                            result,
                            &error,
                            presenter,
                        );
                    }
                    return Err(error);
                }
            }
        }
        PreparedInfobaseCommand::Restore { request, provider } => {
            match infobase_export::preview_infobase_restore(&context, config, &request, &provider) {
                Ok(result) => {
                    if presenter.is_json() {
                        presenter.print_envelope(&Envelope::ok(
                            CommandName::InfobaseRestore.as_str(),
                            0,
                            result,
                        ));
                    } else {
                        render_restore_text(CommandName::InfobaseRestore, &result, presenter);
                    }
                }
                Err(failure) => {
                    let error = failure.error;
                    if let Some(result) = failure.payload {
                        render_restore_failure(
                            CommandName::InfobaseRestore,
                            result,
                            &error,
                            presenter,
                        );
                    }
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

fn execute_infobase_restore(
    config: &AppConfig,
    request: RestoreInfobaseSnapshotRequest,
    prepared: infobase_export::PreparedTransferProvider,
    context: &ExecutionContext,
    presenter: &Presenter,
    clean_before_execution: bool,
) -> Result<(), UseCaseError> {
    let command = CommandName::InfobaseRestore;
    let started = Instant::now();
    let mut workspace_lock_acquired = false;
    let mut dispatched = false;
    let outcome = with_cli_workspace_lock_observed(
        config,
        presenter,
        command,
        clean_before_execution,
        || workspace_lock_acquired = true,
        || {
            dispatched = true;
            info!(
                command = command.as_str(),
                "starting command under workspace lock"
            );
            match infobase_export::execute_infobase_restore(context, config, &request, &prepared) {
                Ok(result) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    if presenter.is_json() {
                        let warnings = result.warnings.clone();
                        let steps = result.steps.clone();
                        let mut envelope = Envelope::ok(command.as_str(), duration_ms, result);
                        envelope.warnings = warnings;
                        envelope.steps = steps;
                        presenter.print_envelope(&envelope);
                    } else {
                        render_restore_text(command, &result, presenter);
                    }
                    Ok(())
                }
                Err(failure) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    let error = failure.error;
                    if presenter.is_json() {
                        match failure.payload {
                            Some(result) => {
                                let warnings = result.warnings.clone();
                                let steps = result.steps.clone();
                                let mut envelope =
                                    failure_envelope(command.as_str(), duration_ms, result, &error);
                                envelope.warnings = warnings;
                                envelope.steps = steps;
                                presenter.print_envelope(&envelope);
                            }
                            None => presenter.print_envelope(&pre_dispatch_error_envelope(
                                command.as_str(),
                                &error,
                            )),
                        }
                    } else {
                        if let Some(result) = failure.payload.as_ref() {
                            render_restore_text(command, result, presenter);
                        }
                        presenter.print_error(&error.to_string());
                    }
                    Err(error)
                }
            }
        },
    );
    if !dispatched {
        if let Err(error) = &outcome {
            let result = restore_pre_dispatch_failure(
                &request,
                Some(prepared.receipt().clone()),
                error,
                infobase_pre_dispatch_execution_phase(workspace_lock_acquired),
            );
            render_restore_failure(command, result, error, presenter);
        }
    }
    outcome
}

fn execute_infobase_configuration_export(
    config: &AppConfig,
    request: ExportConfigurationPackageRequest,
    prepared: infobase_export::PreparedTransferProvider,
    context: &ExecutionContext,
    presenter: &Presenter,
    clean_before_execution: bool,
) -> Result<(), UseCaseError> {
    let command = CommandName::InfobaseConfigurationExport;
    let started = Instant::now();
    let mut workspace_lock_acquired = false;
    let mut dispatched = false;
    let outcome = with_cli_workspace_lock_observed(
        config,
        presenter,
        command,
        clean_before_execution,
        || workspace_lock_acquired = true,
        || {
            dispatched = true;
            info!(
                command = command.as_str(),
                "starting command under workspace lock"
            );
            match infobase_export::execute_configuration_export(
                context, config, &request, &prepared,
            ) {
                Ok(result) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    if presenter.is_json() {
                        let warnings = result.warnings.clone();
                        let steps = result.steps.clone();
                        let mut envelope = Envelope::ok(command.as_str(), duration_ms, result);
                        envelope.warnings = warnings;
                        envelope.steps = steps;
                        presenter.print_envelope(&envelope);
                    } else {
                        render_configuration_export_text(command, &result, presenter);
                    }
                    Ok(())
                }
                Err(failure) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    let error = failure.error;
                    if presenter.is_json() {
                        match failure.payload {
                            Some(result) => {
                                let warnings = result.warnings.clone();
                                let steps = result.steps.clone();
                                let mut envelope =
                                    failure_envelope(command.as_str(), duration_ms, result, &error);
                                envelope.warnings = warnings;
                                envelope.steps = steps;
                                presenter.print_envelope(&envelope);
                            }
                            None => presenter.print_envelope(&pre_dispatch_error_envelope(
                                command.as_str(),
                                &error,
                            )),
                        }
                    } else {
                        if let Some(result) = failure.payload.as_ref() {
                            render_configuration_export_text(command, result, presenter);
                        }
                        presenter.print_error(&error.to_string());
                    }
                    Err(error)
                }
            }
        },
    );
    if !dispatched {
        if let Err(error) = &outcome {
            let result = configuration_pre_dispatch_failure(
                &request,
                Some(prepared.receipt().clone()),
                error,
                infobase_pre_dispatch_execution_phase(workspace_lock_acquired),
            );
            render_configuration_failure(command, result, error, presenter);
        }
    }
    outcome
}

fn execute_infobase_dump(
    config: &AppConfig,
    request: ExportInfobaseSnapshotRequest,
    prepared: infobase_export::PreparedTransferProvider,
    context: &ExecutionContext,
    presenter: &Presenter,
    clean_before_execution: bool,
) -> Result<(), UseCaseError> {
    let command = CommandName::InfobaseDump;
    let started = Instant::now();
    let mut workspace_lock_acquired = false;
    let mut dispatched = false;
    let outcome = with_cli_workspace_lock_observed(
        config,
        presenter,
        command,
        clean_before_execution,
        || workspace_lock_acquired = true,
        || {
            dispatched = true;
            info!(
                command = command.as_str(),
                "starting command under workspace lock"
            );
            match infobase_export::execute_infobase_snapshot(context, config, &request, &prepared) {
                Ok(result) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    if presenter.is_json() {
                        let warnings = result.warnings.clone();
                        let steps = result.steps.clone();
                        let mut envelope = Envelope::ok(command.as_str(), duration_ms, result);
                        envelope.warnings = warnings;
                        envelope.steps = steps;
                        presenter.print_envelope(&envelope);
                    } else {
                        render_snapshot_export_text(command, &result, presenter);
                    }
                    Ok(())
                }
                Err(failure) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    let error = failure.error;
                    if presenter.is_json() {
                        match failure.payload {
                            Some(result) => {
                                let warnings = result.warnings.clone();
                                let steps = result.steps.clone();
                                let mut envelope =
                                    failure_envelope(command.as_str(), duration_ms, result, &error);
                                envelope.warnings = warnings;
                                envelope.steps = steps;
                                presenter.print_envelope(&envelope);
                            }
                            None => presenter.print_envelope(&pre_dispatch_error_envelope(
                                command.as_str(),
                                &error,
                            )),
                        }
                    } else {
                        if let Some(result) = failure.payload.as_ref() {
                            render_snapshot_export_text(command, result, presenter);
                        }
                        presenter.print_error(&error.to_string());
                    }
                    Err(error)
                }
            }
        },
    );
    if !dispatched {
        if let Err(error) = &outcome {
            let result = snapshot_pre_dispatch_failure(
                &request,
                Some(prepared.receipt().clone()),
                error,
                infobase_pre_dispatch_execution_phase(workspace_lock_acquired),
            );
            render_snapshot_failure(command, result, error, presenter);
        }
    }
    outcome
}

fn infobase_pre_dispatch_execution_phase(workspace_lock_acquired: bool) -> InfobaseTransferPhase {
    if workspace_lock_acquired {
        InfobaseTransferPhase::WorkspacePreparation
    } else {
        InfobaseTransferPhase::WorkspaceLock
    }
}

fn annotate_pre_dispatch_failure(execution: &mut ExecutionOutcome<()>, error: &UseCaseError) {
    execution.status = match error.kind() {
        UseCaseErrorKind::InvalidOutput => ExecutionStatus::InvalidOutput,
        UseCaseErrorKind::Cancelled => ExecutionStatus::Cancelled,
        UseCaseErrorKind::TimedOut => ExecutionStatus::TimedOut,
        _ => ExecutionStatus::Failed,
    };
    execution.errors.push(ExecutionError::new(
        execution_step_code(error.kind()),
        error.message(),
    ));
}

/// Код шага исполнителя: свой словарь, едущий внутри `data.execution.errors[]`.
///
/// Имена совпадают с кодами конверта, но поля разные, и различать их должен код, а не
/// читатель: конверт стал точнее — у рода `capability` там четыре кода, — а шаг остаётся
/// при прежнем словаре, потому что его читает другой потребитель.
const fn execution_step_code(kind: UseCaseErrorKind) -> &'static str {
    match kind {
        UseCaseErrorKind::Capability(_) => "capability_unavailable",
        UseCaseErrorKind::Environment => "environment_unavailable",
        UseCaseErrorKind::WorkspaceBusy => "workspace_busy",
        UseCaseErrorKind::InvalidOutput => "invalid_output",
        UseCaseErrorKind::Cancelled => "cancelled",
        UseCaseErrorKind::TimedOut => "timed_out",
        UseCaseErrorKind::Validation => "invalid_argument",
        UseCaseErrorKind::Runtime => "runtime_failure",
        UseCaseErrorKind::Platform => "platform_failure",
    }
}

fn configuration_pre_dispatch_failure(
    request: &ExportConfigurationPackageRequest,
    selection: Option<ProviderReceipt>,
    error: &UseCaseError,
    phase: InfobaseTransferPhase,
) -> ExportConfigurationPackageResult {
    let mut result = ExportConfigurationPackageResult::new(request.clone(), selection);
    annotate_pre_dispatch_failure(&mut result.execution, error);
    result.steps.push(
        StepResult::failed(phase.as_str(), phase.kind(), 0)
            .with_message(error.message().to_owned()),
    );
    result
}

fn snapshot_pre_dispatch_failure(
    request: &ExportInfobaseSnapshotRequest,
    selection: Option<ProviderReceipt>,
    error: &UseCaseError,
    phase: InfobaseTransferPhase,
) -> ExportInfobaseSnapshotResult {
    let mut result = ExportInfobaseSnapshotResult::new(request.clone(), selection);
    annotate_pre_dispatch_failure(&mut result.execution, error);
    result.steps.push(
        StepResult::failed(phase.as_str(), phase.kind(), 0)
            .with_message(error.message().to_owned()),
    );
    result
}

fn restore_pre_dispatch_failure(
    request: &RestoreInfobaseSnapshotRequest,
    selection: Option<ProviderReceipt>,
    error: &UseCaseError,
    phase: InfobaseTransferPhase,
) -> RestoreInfobaseSnapshotResult {
    let mut result = RestoreInfobaseSnapshotResult::new(request.clone(), selection);
    annotate_pre_dispatch_failure(&mut result.execution, error);
    result.steps.push(
        StepResult::failed(phase.as_str(), phase.kind(), 0)
            .with_message(error.message().to_owned()),
    );
    result
}

fn render_restore_failure(
    command: CommandName,
    result: RestoreInfobaseSnapshotResult,
    error: &UseCaseError,
    presenter: &Presenter,
) {
    if presenter.is_json() {
        let warnings = result.warnings.clone();
        let steps = result.steps.clone();
        let mut envelope = failure_envelope(command.as_str(), 0, result, error);
        envelope.warnings = warnings;
        envelope.steps = steps;
        presenter.print_envelope(&envelope);
    } else {
        render_restore_text(command, &result, presenter);
        presenter.print_error(&error.to_string());
    }
}

fn render_configuration_failure(
    command: CommandName,
    result: ExportConfigurationPackageResult,
    error: &UseCaseError,
    presenter: &Presenter,
) {
    if presenter.is_json() {
        let warnings = result.warnings.clone();
        let steps = result.steps.clone();
        let mut envelope = failure_envelope(command.as_str(), 0, result, error);
        envelope.warnings = warnings;
        envelope.steps = steps;
        presenter.print_envelope(&envelope);
    } else {
        render_configuration_export_text(command, &result, presenter);
        presenter.print_error(&error.to_string());
    }
}

fn render_snapshot_failure(
    command: CommandName,
    result: ExportInfobaseSnapshotResult,
    error: &UseCaseError,
    presenter: &Presenter,
) {
    if presenter.is_json() {
        let warnings = result.warnings.clone();
        let steps = result.steps.clone();
        let mut envelope = failure_envelope(command.as_str(), 0, result, error);
        envelope.warnings = warnings;
        envelope.steps = steps;
        presenter.print_envelope(&envelope);
    } else {
        render_snapshot_export_text(command, &result, presenter);
        presenter.print_error(&error.to_string());
    }
}

struct InfobaseExportText<'a> {
    command: &'a str,
    label: &'a str,
    state: Option<&'a str>,
    subject: String,
    artifact_kind: &'a str,
    execution_status: &'a str,
    /// Field name for `path`: an export names its output, a restore names its input.
    path_label: &'a str,
    path: &'a Path,
    provider: Option<&'a ProviderReceipt>,
    /// Field name for `applied`: an export publishes, a restore loads.
    applied_label: &'a str,
    applied: bool,
    target_state: &'a str,
    warnings: &'a [String],
    mode: crate::domain::infobase_export::InfobaseExportMode,
    provider_dispatched: Option<bool>,
}

fn render_configuration_export_text(
    command: CommandName,
    result: &ExportConfigurationPackageResult,
    presenter: &Presenter,
) {
    render_infobase_export_text(
        InfobaseExportText {
            command: command.as_str(),
            label: "Configuration package export",
            state: Some(result.state.as_str()),
            subject: render_configuration_subject(&result.subject),
            artifact_kind: result.artifact_kind.as_str(),
            execution_status: execution_status_label(result.execution.status),
            path_label: "output",
            path: &result.output,
            provider: result.provider.as_ref(),
            applied_label: "published",
            applied: result.published,
            target_state: export_target_state_label(result.target_state),
            warnings: &result.warnings,
            mode: result.mode,
            provider_dispatched: result.provider_dispatched,
        },
        presenter,
    );
}

fn render_snapshot_export_text(
    command: CommandName,
    result: &ExportInfobaseSnapshotResult,
    presenter: &Presenter,
) {
    render_infobase_export_text(
        InfobaseExportText {
            command: command.as_str(),
            label: "Infobase DT export",
            state: None,
            subject: "infobase".to_owned(),
            artifact_kind: result.artifact_kind.as_str(),
            execution_status: execution_status_label(result.execution.status),
            path_label: "output",
            path: &result.output,
            provider: result.provider.as_ref(),
            applied_label: "published",
            applied: result.published,
            target_state: export_target_state_label(result.target_state),
            warnings: &result.warnings,
            mode: result.mode,
            provider_dispatched: result.provider_dispatched,
        },
        presenter,
    );
}

fn render_restore_text(
    command: CommandName,
    result: &RestoreInfobaseSnapshotResult,
    presenter: &Presenter,
) {
    render_infobase_export_text(
        InfobaseExportText {
            command: command.as_str(),
            label: "Infobase DT restore",
            state: None,
            subject: format!("infobase:{}", result.target_mode.as_str()),
            artifact_kind: result.artifact_kind.as_str(),
            execution_status: execution_status_label(result.execution.status),
            path_label: "input",
            path: &result.input,
            provider: result.provider.as_ref(),
            applied_label: "restored",
            applied: result.restored,
            target_state: export_target_state_label(result.target_state),
            warnings: &result.warnings,
            mode: result.mode,
            provider_dispatched: result.provider_dispatched,
        },
        presenter,
    );
}

fn render_infobase_export_text(view: InfobaseExportText<'_>, presenter: &Presenter) {
    let InfobaseExportText {
        command,
        label,
        state,
        subject,
        artifact_kind,
        execution_status,
        path_label,
        path,
        provider,
        applied_label,
        applied,
        target_state,
        warnings,
        mode,
        provider_dispatched,
    } = view;
    let status = if applied
        || (mode == crate::domain::infobase_export::InfobaseExportMode::Preview
            && execution_status == "succeeded")
    {
        TimelineStatus::Succeeded
    } else {
        TimelineStatus::Failed
    };
    let provider_line = match provider {
        Some(receipt) => match receipt.selected {
            Some(selected) => format!(
                "provider: {selected} ({})",
                provider_origin_label(&receipt.origin)
            ),
            None => "provider: none is ready".to_owned(),
        },
        None => "provider: not selected".to_owned(),
    };
    let mut details = vec![
        format!("command: {command}"),
        format!(
            "mode: {}",
            match mode {
                crate::domain::infobase_export::InfobaseExportMode::Preview => "preview",
                crate::domain::infobase_export::InfobaseExportMode::Apply => "apply",
            }
        ),
        format!("subject: {subject}"),
        format!("artifact kind: {artifact_kind}"),
        provider_line,
        format!("execution status: {execution_status}"),
        format!("{applied_label}: {applied}"),
        format!("target state: {target_state}"),
        format!("{path_label}: {}", path.display()),
    ];
    if let Some(provider_dispatched) = provider_dispatched {
        details.insert(2, format!("provider dispatched: {provider_dispatched}"));
    }
    for skipped in provider
        .map(|receipt| receipt.skipped.as_slice())
        .unwrap_or_default()
    {
        details.push(format!(
            "[skipped:{}] {}",
            skipped.provider.as_str(),
            skipped.reason
        ));
    }
    if let Some(state) = state {
        details.insert(0, format!("state: {state}"));
    }
    details.extend(warnings.iter().map(|warning| format!("warning: {warning}")));
    presenter.print_timeline(&[TimelineItem::new(status, label).with_detail(details.join("\n"))]);
}

/// Строки квитанции о выборе исполнителя — одни и те же у всех команд.
fn provider_receipt_details(receipt: Option<&ProviderReceipt>) -> Vec<String> {
    let Some(receipt) = receipt else {
        return Vec::new();
    };
    let mut details = vec![match receipt.selected {
        Some(selected) => format!(
            "provider: {selected} ({})",
            provider_origin_label(&receipt.origin)
        ),
        None => "provider: none is ready".to_owned(),
    }];
    for skipped in &receipt.skipped {
        details.push(format!(
            "[skipped:{}] {}",
            skipped.provider.as_str(),
            skipped.reason
        ));
    }
    details
}

fn provider_origin_label(origin: &crate::domain::capability::ProviderOrigin) -> String {
    match origin {
        crate::domain::capability::ProviderOrigin::Default => "default".to_owned(),
        crate::domain::capability::ProviderOrigin::Override { file } => {
            format!("providers.* in {file}")
        }
    }
}

fn export_target_state_label(
    state: crate::domain::infobase_export::ExportTargetState,
) -> &'static str {
    use crate::domain::infobase_export::ExportTargetState;
    match state {
        ExportTargetState::Unchanged => "unchanged",
        ExportTargetState::Created => "created",
        ExportTargetState::Replaced => "replaced",
        ExportTargetState::Restored => "restored",
        ExportTargetState::Uncertain => "uncertain",
    }
}

fn render_configuration_subject(
    subject: &crate::domain::infobase_export::ConfigurationSubject,
) -> String {
    match subject {
        crate::domain::infobase_export::ConfigurationSubject::Main => "main".to_owned(),
        crate::domain::infobase_export::ConfigurationSubject::Extension { name } => {
            format!("extension:{name}")
        }
    }
}

fn execution_status_label(status: crate::domain::execution::ExecutionStatus) -> &'static str {
    match status {
        crate::domain::execution::ExecutionStatus::Succeeded => "succeeded",
        crate::domain::execution::ExecutionStatus::Failed => "failed",
        crate::domain::execution::ExecutionStatus::Cancelled => "cancelled",
        crate::domain::execution::ExecutionStatus::TimedOut => "timed_out",
        crate::domain::execution::ExecutionStatus::InvalidOutput => "invalid_output",
    }
}

fn execute_convert(
    config: &AppConfig,
    args: &ConvertArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_convert_request(args, dry_run);
    if let Err(error) = convert_sources::preflight_validate(config, &request) {
        return Err(render_pre_dispatch_error(
            presenter,
            CommandName::Convert,
            error,
        ));
    }
    let context = cli_context(config, CommandName::Convert, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Convert,
        clean_before_execution,
        dry_run,
        || match convert_sources::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Convert.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_convert_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        presenter.print_envelope(&failure_envelope(
                            CommandName::Convert.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        ));
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_convert_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_artifacts(
    config: &AppConfig,
    args: &ArtifactsArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_artifacts_request_with_config(config, args, dry_run)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Artifacts, error))?;
    let context = cli_context(config, CommandName::Artifacts, cancellation);
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Artifacts,
        clean_before_execution,
        dry_run,
        || match artifacts::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    let envelope = build_artifacts_envelope(&result);
                    presenter.print_envelope(&envelope);
                } else {
                    render_artifacts_text(&result, presenter, true);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        let envelope = with_cli_error(build_artifacts_envelope(&result), &error);
                        presenter.print_envelope(&envelope);
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_artifacts_text(result, presenter, false);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_syntax(
    config: &AppConfig,
    args: &SyntaxArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let context = cli_context(config, CommandName::Syntax, cancellation);
    let request = map_syntax_request(config, args, dry_run)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Syntax, error))?;
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Syntax,
        clean_before_execution,
        dry_run,
        || match check_syntax::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Syntax.as_str(),
                        result.duration_ms,
                        result,
                    ));
                } else {
                    render_syntax_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    if let Some(result) = failure.payload {
                        presenter.print_envelope(&failure_envelope(
                            CommandName::Syntax.as_str(),
                            result.duration_ms,
                            result,
                            &error,
                        ));
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_syntax_text(result, presenter);
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

fn execute_launch(
    config: &AppConfig,
    args: &LaunchArgs,
    presenter: &Presenter,
    clean_before_execution: bool,
    dry_run: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let request = map_launch_request(args, dry_run)
        .map_err(|error| render_pre_dispatch_error(presenter, CommandName::Launch, error))?;
    let context = cli_context(config, CommandName::Launch, cancellation);
    let started = Instant::now();
    with_cli_workspace_lock(
        config,
        presenter,
        CommandName::Launch,
        clean_before_execution,
        dry_run,
        || match launch_app::execute(&context, config, &request) {
            Ok(result) => {
                if presenter.is_json() {
                    presenter.print_envelope(&Envelope::ok(
                        CommandName::Launch.as_str(),
                        started.elapsed().as_millis() as u64,
                        result,
                    ));
                } else {
                    render_launch_text(&result, presenter);
                }
                Ok(())
            }
            Err(failure) => {
                let error = failure.error;
                if presenter.is_json() {
                    match failure.payload {
                        Some(result) => presenter.print_envelope(&failure_envelope(
                            CommandName::Launch.as_str(),
                            started.elapsed().as_millis() as u64,
                            result,
                            &error,
                        )),
                        None => presenter.print_envelope(&failure_envelope(
                            CommandName::Launch.as_str(),
                            started.elapsed().as_millis() as u64,
                            crate::cli::output::RefusalData {
                                message: error.message().to_owned(),
                            },
                            &error,
                        )),
                    }
                } else {
                    if let Some(result) = failure.payload.as_ref() {
                        render_launch_text_with_status(
                            result,
                            presenter,
                            TimelineStatus::Failed,
                            "Launch",
                        );
                    }
                    presenter.print_error(&error.to_string());
                }
                Err(error)
            }
        },
    )
}

/// Граница владения `workPath` у CLI-адаптера.
///
/// `preview` говорит, что запуск ничего в `workPath` не изменит. Тогда блокировка не
/// берётся: держать её не за что, а вред настоящий — «покажи план» упиралось бы в
/// занятое пространство и отказывало `workspace_busy`, а два одновременных превью
/// выстраивались бы в очередь. Прецедент — `infobase ... --dry-run` по DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED,
/// который возвращается до блокировок вообще.
///
/// Решение живёт здесь, а не в отдельном помощнике рядом: у границы один владелец,
/// иначе её легко обойти новым вызовом. Спор превью с очисткой — вопрос не границы, а
/// двух глобальных ключей, и отвечает на него `app::run` до загрузки конфига.
pub(crate) fn with_cli_workspace_lock<T>(
    config: &AppConfig,
    presenter: &Presenter,
    command: CommandName,
    clean_before_execution: bool,
    preview: bool,
    run: impl FnOnce() -> Result<T, UseCaseError>,
) -> Result<T, UseCaseError> {
    if preview {
        // Чистка меняет `workPath` и с превью не сочетается; спор двух глобальных ключей
        // разрешается один раз на запуске (`app::run`) — до того, как каталог тронут, — и
        // здесь уже не повторяется.
        debug_assert!(
            !clean_before_execution,
            "очистка с превью отклонена на запуске"
        );
        return run();
    }
    with_cli_workspace_lock_observed(
        config,
        presenter,
        command,
        clean_before_execution,
        || {},
        run,
    )
}

fn with_cli_workspace_lock_observed<T>(
    config: &AppConfig,
    presenter: &Presenter,
    command: CommandName,
    clean_before_execution: bool,
    workspace_lock_acquired: impl FnOnce(),
    run: impl FnOnce() -> Result<T, UseCaseError>,
) -> Result<T, UseCaseError> {
    let busy_policy = if matches!(
        command,
        CommandName::InfobaseConfigurationExport
            | CommandName::InfobaseDump
            | CommandName::Bootstrap
    ) {
        WorkspaceBusyPolicy::Typed
    } else {
        WorkspaceBusyPolicy::LegacyRuntime
    };
    let result = dispatch_with_workspace_lock_policy(
        config,
        command,
        busy_policy,
        || {
            workspace_lock_acquired();
            if clean_before_execution {
                clean_platform_logs_under_lock(config)
            } else {
                Ok(())
            }
        },
        run,
    );
    result.map_err(|error| {
        if matches!(busy_policy, WorkspaceBusyPolicy::Typed) {
            error
        } else {
            render_pre_dispatch_error(presenter, command, error)
        }
    })?
}

fn clean_platform_logs_under_lock(config: &AppConfig) -> Result<(), UseCaseError> {
    platform_logs_dir(&config.work_path)
        .and_then(|dir| clean_dir(&dir))
        .map_err(|error| {
            UseCaseError::from(AppError::Runtime(format!(
                "failed to clean platform logs: {error}"
            )))
        })
}

fn render_pre_dispatch_error(
    presenter: &Presenter,
    command: CommandName,
    error: impl Into<UseCaseError>,
) -> UseCaseError {
    let error = error.into();
    print_command_use_case_error(presenter, command, &error);
    error
}

fn map_build_request(args: &BuildArgs, dry_run: bool) -> BuildRequest {
    BuildRequest {
        dry_run,
        full_rebuild: args.full_rebuild,
        source_set: args.source_set.clone(),
    }
}

fn map_extensions_request(args: &ExtensionsArgs, dry_run: bool) -> ConfigureExtensionsRequest {
    ConfigureExtensionsRequest {
        names: args.names.clone(),
        installed_names: args.installed_names.clone(),
        dry_run,
    }
}

fn map_tools_download_target(args: &ToolsDownloadArgs) -> ToolDownloadTarget {
    match &args.command {
        ToolsDownloadCommand::Yaxunit(_) => ToolDownloadTarget::Yaxunit,
        ToolsDownloadCommand::Vanessa(_) => ToolDownloadTarget::VanessaAutomationSingle,
        ToolsDownloadCommand::ClientMcp(_) => ToolDownloadTarget::ClientMcp,
    }
}

fn map_tool_extension_mode(args: &ToolsDownloadArgs) -> ToolExtensionInstallMode {
    match &args.command {
        ToolsDownloadCommand::Yaxunit(args) | ToolsDownloadCommand::ClientMcp(args) => {
            if args.sources {
                ToolExtensionInstallMode::Sources
            } else {
                ToolExtensionInstallMode::Artifacts
            }
        }
        ToolsDownloadCommand::Vanessa(_) => ToolExtensionInstallMode::Artifacts,
    }
}

fn map_tools_download_force(args: &ToolsDownloadArgs) -> bool {
    match &args.command {
        ToolsDownloadCommand::Yaxunit(args) | ToolsDownloadCommand::ClientMcp(args) => args.force,
        ToolsDownloadCommand::Vanessa(args) => args.force,
    }
}

fn map_test_request(config: &AppConfig, args: &TestArgs) -> Result<TestRequest, UseCaseError> {
    let client_mode = map_test_client_mode(args.client_mode.as_deref())?;
    let build_policy = if args.no_build {
        crate::use_cases::request::TestBuildPolicy::Skip
    } else {
        crate::use_cases::request::TestBuildPolicy::BuildFirst
    };
    match &args.runner {
        TestRunner::Yaxunit(TestYaxunitArgs { scope }) => {
            let scope = map_yaxunit_scope(scope)?;
            Ok(TestRequest {
                execution: build_yaxunit_execution(config, &args.launch, client_mode)?,
                full: args.full,
                build_policy,
                scope,
            })
        }
        TestRunner::Va(_) => Ok(TestRequest {
            execution: build_vanessa_execution(config, &args.launch, client_mode)?,
            full: args.full,
            build_policy,
            scope: TestScopeRequest::All,
        }),
    }
}

fn effective_test_config(config: &AppConfig, args: &TestArgs) -> Result<AppConfig, UseCaseError> {
    let mut config = config.clone();
    if let TestRunner::Va(va_args) = &args.runner {
        apply_vanessa_cli_profile_overrides(&mut config, va_args)?;
    }
    Ok(config)
}

fn apply_vanessa_cli_profile_overrides(
    config: &mut AppConfig,
    args: &TestVaArgs,
) -> Result<(), UseCaseError> {
    if !args.has_profile_overrides() {
        return Ok(());
    }
    let profile_id = config.tests.va.profile.clone().ok_or_else(|| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            "tests.va.profile is not configured",
        )
    })?;
    let profile = config
        .tests
        .va
        .profiles
        .get_mut(&profile_id)
        .ok_or_else(|| {
            UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("unknown Vanessa Automation profile '{profile_id}'"),
            )
        })?;
    if !args.features_to_run.is_empty() {
        profile.features_to_run.clone_from(&args.features_to_run);
    }
    if !args.filter_tags.is_empty() {
        profile.filter_tags.clone_from(&args.filter_tags);
    }
    if !args.ignore_tags.is_empty() {
        profile.ignore_tags.clone_from(&args.ignore_tags);
    }
    if !args.scenario_filter.is_empty() {
        profile.scenario_filter.clone_from(&args.scenario_filter);
    }
    Ok(())
}

fn map_test_client_mode(
    client_mode: Option<&str>,
) -> Result<Option<LaunchClientModeRequest>, UseCaseError> {
    Ok(match client_mode {
        Some("designer") => Some(LaunchClientModeRequest::Designer),
        Some("thin") => Some(LaunchClientModeRequest::Thin),
        Some("thick") => Some(LaunchClientModeRequest::Thick),
        Some("ordinary") => Some(LaunchClientModeRequest::Ordinary),
        Some(other) => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("unsupported test client mode: {other}"),
            ));
        }
        None => None,
    })
}

fn map_yaxunit_scope(scope: &TestScope) -> Result<TestScopeRequest, UseCaseError> {
    Ok(match scope {
        TestScope::All => TestScopeRequest::All,
        TestScope::Module { name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "test module requires a non-empty module name",
                ));
            }
            TestScopeRequest::Module {
                name: trimmed.to_owned(),
            }
        }
    })
}

fn build_yaxunit_execution(
    config: &AppConfig,
    launch_args: &TestLaunchOptionsArgs,
    client_mode: Option<LaunchClientModeRequest>,
) -> Result<crate::domain::runner::ScenarioExecutionRequest, UseCaseError> {
    validate_test_launch_options(launch_args)?;
    let mut execution = TestRequest::default_execution();
    execution.timeouts = effective_test_timeouts(
        config.tests.execution_timeout_seconds,
        &config.tests.yaxunit.timeouts,
    );
    execution.launch = map_test_launch_options(launch_args)?;
    execution.client_mode = client_mode.or(Some(LaunchClientModeRequest::Thin));
    execution.launch.c = Some("RunUnitTests={config_path}".to_owned());
    Ok(execution)
}

fn build_vanessa_execution(
    config: &AppConfig,
    launch_args: &TestLaunchOptionsArgs,
    client_mode: Option<LaunchClientModeRequest>,
) -> Result<crate::domain::runner::ScenarioExecutionRequest, UseCaseError> {
    validate_test_launch_options(launch_args)?;
    let profile_id = config.tests.va.profile.as_deref().ok_or_else(|| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            "tests.va.profile is not configured",
        )
    })?;
    if !config.tests.va.profiles.contains_key(profile_id) {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            format!("unknown Vanessa Automation profile '{profile_id}'"),
        ));
    }
    if !is_safe_path_segment(profile_id) {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            format!("tests.va.profile contains unsafe path characters: {profile_id}"),
        ));
    }
    if config.tools.va.epf_path.is_none() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "tools.va.epf_path is not configured",
        ));
    }
    if config.tests.va.params_path.is_none() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "tests.va.params_path is not configured",
        ));
    }

    let mut execution = crate::domain::runner::ScenarioExecutionRequest {
        profile: RunnerProfile {
            id: profile_id.to_owned(),
            kind: RunnerKind::Vanessa,
            output_formats: vec![
                RunnerOutputFormat::JunitXml,
                RunnerOutputFormat::PlainTextLog,
            ],
            backend_hint: Some("enterprise".to_owned()),
        },
        client_mode: Some(LaunchClientModeRequest::Thin),
        timeouts: effective_test_timeouts(
            config.tests.execution_timeout_seconds,
            &config.tests.va.timeouts,
        ),
        policy: ExecutionPolicy {
            retain_artifacts_on_failure: true,
            retain_artifacts_on_success: false,
        },
        launch: LaunchOptions::default(),
    };
    execution.launch = map_test_launch_options(launch_args)?;
    execution.client_mode = client_mode.or(Some(LaunchClientModeRequest::Thin));
    execution.launch.c = Some("StartFeaturePlayer;VAParams={params_path}".to_owned());
    execution.launch.execute = Some("{epf_path}".to_owned());
    Ok(execution)
}

fn map_test_launch_options(args: &TestLaunchOptionsArgs) -> Result<LaunchOptions, UseCaseError> {
    Ok(LaunchOptions {
        c: None,
        execute: None,
        use_privileged_mode: args.use_privileged_mode,
        out: None,
        internal_out: None,
        raw_args: args.raw_keys.clone(),
        external_epf_wait: None,
    })
}

fn validate_test_launch_options(args: &TestLaunchOptionsArgs) -> Result<(), UseCaseError> {
    if args
        .raw_keys
        .iter()
        .any(|raw| is_reserved_raw_launch_key(raw))
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "test manages /C, /Execute, and /Out internally and does not support raw /C, /Execute, or /Out keys",
        ));
    }
    Ok(())
}

/// Builds the execution context for a CLI command.
///
/// A public command carries no deadline: see DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE.
/// What bounds a step is the step's own cap, and what ends a run early is the operator's
/// interrupt, which reaches the context as cancellation.
///
/// The EDT step cap is one such declared cap, and it is read here so that both transports
/// bound an EDT subprocess by the same configured key. Before this it reached use cases
/// from MCP only, and a one-shot `1cedtcli` started from the CLI was bounded by nothing but
/// the command deadline that no longer exists.
fn cli_context(
    config: &AppConfig,
    command: CommandName,
    cancellation: CancellationToken,
) -> ExecutionContext {
    ExecutionContext::cli(command)
        .with_edt_timeout(Some(Duration::from_millis(
            config.tools.edt_cli.command_timeout_ms,
        )))
        .with_cancellation(cancellation)
}

fn map_load_request(args: &LoadArgs, dry_run: bool) -> Result<LoadRequest, UseCaseError> {
    Ok(LoadRequest {
        dry_run,
        mode: match args.mode.as_str() {
            "load" => LoadMode::Load,
            "combine" | "merge" => LoadMode::Merge,
            "update" => LoadMode::Update,
            other => {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    format!("unsupported load mode: {other}"),
                ));
            }
        },
        artifact_path: args.path.clone(),
        settings_path: args.settings.clone(),
        vendor_name: args.vendor_name.clone(),
        extension: args.extension.clone(),
    })
}

fn map_dump_request(args: &DumpArgs, dry_run: bool) -> Result<DumpRequest, UseCaseError> {
    Ok(DumpRequest {
        dry_run,
        mode: parse_required_dump_mode(&args.mode)?,
        source_set: args.source_set.clone(),
        extension: args.extension.clone(),
        objects: args.objects.clone(),
        discard_uncommitted: args.discard_uncommitted,
    })
}

fn map_convert_request(args: &ConvertArgs, dry_run: bool) -> ConvertRequest {
    ConvertRequest {
        scope: match args.source_set.as_deref() {
            Some(name) => ConvertScopeRequest::SourceSet {
                name: name.to_owned(),
            },
            None => ConvertScopeRequest::All,
        },
        output_root: args.output.clone(),
        dry_run,
        discard_uncommitted: args.discard_uncommitted,
    }
}

fn map_artifacts_request_with_config(
    config: &AppConfig,
    args: &ArtifactsArgs,
    dry_run: bool,
) -> Result<ArtifactsRequest, UseCaseError> {
    let mode = match (args.source_set.as_deref(), args.extension.is_some()) {
        (_, true) => ArtifactsModeRequest::ExtensionCfe,
        (Some(source_set_name), false) => {
            let source_set = config
                .source_sets
                .iter()
                .find(|source_set| source_set.name == source_set_name)
                .ok_or_else(|| {
                    UseCaseError::new(
                        UseCaseErrorKind::Validation,
                        format!("unknown source-set '{source_set_name}'"),
                    )
                })?;
            match source_set.purpose {
                SourceSetPurpose::Configuration => ArtifactsModeRequest::ConfigurationCf,
                SourceSetPurpose::Extension => ArtifactsModeRequest::ExtensionCfe,
                SourceSetPurpose::ExternalDataProcessors => {
                    ArtifactsModeRequest::ExternalDataProcessorEpf
                }
                SourceSetPurpose::ExternalReports => ArtifactsModeRequest::ExternalReportErf,
            }
        }
        (None, false) => ArtifactsModeRequest::ConfigurationCf,
    };

    Ok(ArtifactsRequest {
        dry_run,
        execution: ArtifactsRequest::default_execution(mode),
        mode,
        output_path: args.output.clone(),
        source_set: args.source_set.clone(),
        extension: args.extension.clone(),
    })
}

/// Ветку выбирает формат проекта, а не подкоманда: проверка одна, а чем её выполнить —
/// свойство проекта. Прежние имена приняты один цикл и держат своё утверждение о ветке:
/// `check edt` в проекте формата платформы отказывает, как отказывал, — синоним не меняет
/// инструмент молча.
fn map_syntax_request(
    config: &AppConfig,
    args: &SyntaxArgs,
    dry_run: bool,
) -> Result<SyntaxRequest, UseCaseError> {
    if let Some(message) = args.keys_next_to_a_previous_name() {
        return Err(UseCaseError::new(UseCaseErrorKind::Validation, message));
    }
    let target = match &args.target {
        Some(SyntaxTarget::DesignerConfig(modes)) => {
            SyntaxTargetRequest::DesignerConfig(map_designer_config_request(modes)?)
        }
        Some(SyntaxTarget::DesignerModules(modules)) => {
            SyntaxTargetRequest::DesignerConfig(map_designer_modules_request(modules)?)
        }
        Some(SyntaxTarget::Edt { projects }) => SyntaxTargetRequest::Edt {
            projects: projects.clone(),
        },
        None if config.format == SourceFormat::Edt => {
            // Ключ, которого ветка не исполняет, отвергается, а не игнорируется.
            if args.modes != DesignerConfigSyntaxArgs::default() {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "the project format is EDT, and the check runs EDT validation: modes of /CheckConfig are not executed there. Name the project with --project or drop the keys",
                ));
            }
            SyntaxTargetRequest::Edt {
                projects: args.projects.clone(),
            }
        }
        None => {
            if !args.projects.is_empty() {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "--project names an EDT project, and the project format is DESIGNER: the check runs /CheckConfig there. Drop the key",
                ));
            }
            let named = map_designer_config_request(&args.modes)?;
            // Ключей не назвали — выполняется профиль по умолчанию: пустая `/CheckConfig`
            // не проверяет ничего и отвечает «чисто», а команда обещает проверку. Прежние
            // имена сюда не попадают: у них свой состав и своя проверка режимов.
            let request = if named.names_no_mode() {
                DesignerConfigSyntaxRequest::default_profile(named.extension_scope().clone())
            } else {
                named
            };
            SyntaxTargetRequest::DesignerConfig(request)
        }
    };
    Ok(SyntaxRequest { target, dry_run })
}

fn map_designer_config_request(
    args: &DesignerConfigSyntaxArgs,
) -> Result<DesignerConfigSyntaxRequest, UseCaseError> {
    Ok(DesignerConfigSyntaxRequest::new(
        DesignerConfigChecks::new(
            [
                args.config_log_integrity
                    .then_some(DesignerConfigCheck::ConfigLogIntegrity),
                args.incorrect_references
                    .then_some(DesignerConfigCheck::IncorrectReferences),
                args.mobile_client_digi_sign
                    .then_some(DesignerConfigCheck::MobileClientDigiSign),
                args.distributive_modules
                    .then_some(DesignerConfigCheck::DistributiveModules),
                args.unreference_procedures
                    .then_some(DesignerConfigCheck::UnreferenceProcedures),
                args.handlers_existence
                    .then_some(DesignerConfigCheck::HandlersExistence),
                args.empty_handlers
                    .then_some(DesignerConfigCheck::EmptyHandlers),
                args.unsupported_functional
                    .then_some(DesignerConfigCheck::UnsupportedFunctional),
            ]
            .into_iter()
            .flatten(),
        ),
        DesignerClientScopes::new(
            [
                args.thin_client.then_some(DesignerClientScope::ThinClient),
                args.web_client.then_some(DesignerClientScope::WebClient),
                args.mobile_client
                    .then_some(DesignerClientScope::MobileClient),
                args.server.then_some(DesignerClientScope::Server),
                args.external_connection
                    .then_some(DesignerClientScope::ExternalConnection),
                args.external_connection_server
                    .then_some(DesignerClientScope::ExternalConnectionServer),
                args.mobile_app_client
                    .then_some(DesignerClientScope::MobileAppClient),
                args.mobile_app_server
                    .then_some(DesignerClientScope::MobileAppServer),
                args.thick_client_managed_application
                    .then_some(DesignerClientScope::ThickClientManagedApplication),
                args.thick_client_server_managed_application
                    .then_some(DesignerClientScope::ThickClientServerManagedApplication),
                args.thick_client_ordinary_application
                    .then_some(DesignerClientScope::ThickClientOrdinaryApplication),
                args.thick_client_server_ordinary_application
                    .then_some(DesignerClientScope::ThickClientServerOrdinaryApplication),
            ]
            .into_iter()
            .flatten(),
        ),
        crate::use_cases::request::ExtendedModulesPolicy::from_cli_flags(
            args.extended_modules_check,
            args.check_use_synchronous_calls,
            args.check_use_modality,
        )?,
        SyntaxExtensionScope::new(args.extension.clone(), args.all_extensions),
    ))
}

/// Прежнее имя `designer-modules` исполняется той же `/CheckConfig`: её режимы покрывают
/// режимы проверки модулей целиком. Проверки конфигурации остаются пустыми, а требование
/// «хотя бы один режим» сохраняется: синоним держится один цикл ровно тем, чем был, и
/// профиль по умолчанию сюда не подмешивается.
fn map_designer_modules_request(
    args: &DesignerModulesSyntaxArgs,
) -> Result<DesignerConfigSyntaxRequest, UseCaseError> {
    let request = DesignerConfigSyntaxRequest::new(
        DesignerConfigChecks::new([]),
        DesignerClientScopes::new(
            [
                args.thin_client.then_some(DesignerClientScope::ThinClient),
                args.web_client.then_some(DesignerClientScope::WebClient),
                args.server.then_some(DesignerClientScope::Server),
                args.external_connection
                    .then_some(DesignerClientScope::ExternalConnection),
                args.thick_client_ordinary_application
                    .then_some(DesignerClientScope::ThickClientOrdinaryApplication),
                args.mobile_app_client
                    .then_some(DesignerClientScope::MobileAppClient),
                args.mobile_app_server
                    .then_some(DesignerClientScope::MobileAppServer),
                args.mobile_client
                    .then_some(DesignerClientScope::MobileClient),
            ]
            .into_iter()
            .flatten(),
        ),
        crate::use_cases::request::ExtendedModulesPolicy::basic(args.extended_modules_check),
        SyntaxExtensionScope::new(args.extension.clone(), args.all_extensions),
    );
    if request.names_no_mode() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            crate::use_cases::request::MODULES_WITHOUT_MODES_ERROR,
        ));
    }
    Ok(request)
}

fn map_launch_request(args: &LaunchArgs, dry_run: bool) -> Result<LaunchRequest, UseCaseError> {
    let mut target = parse_launch_target(&args.target, "mode", LaunchModeAliases::Cli)?;
    let client_mcp = if matches!(
        target,
        crate::use_cases::request::LaunchTargetRequest::Enterprise(
            crate::use_cases::request::EnterpriseLaunchTarget::ClientMcp { .. }
        )
    ) {
        let mode = map_mcp_client_mode(args.mcp_mode.as_deref())?;
        target = crate::use_cases::request::LaunchTargetRequest::client_mcp_with_mode(mode);
        Some(map_mcp_options(args)?)
    } else {
        if args.mcp_config.is_some()
            || args.mcp_port.is_some()
            || args.mcp_mode.is_some()
            || args.mcp_scenario.is_some()
            || args.wait_ready
        {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--mcp-config, --mcp-port, --mode, --wait-ready, and MCP_SCENARIO are supported only for `launch mcp`",
            ));
        }
        None
    };
    Ok(LaunchRequest {
        target,
        launch: map_direct_launch_options(target, &args.launch, client_mcp.is_some())?,
        client_mcp,
        via: map_launch_via(args.via.as_deref())?,
        dry_run,
    })
}

fn map_direct_launch_options(
    target: crate::use_cases::request::LaunchTargetRequest,
    args: &DirectLaunchOptionsArgs,
    is_client_mcp: bool,
) -> Result<LaunchOptions, UseCaseError> {
    if is_client_mcp {
        if args.wait_for_exit || args.wait_timeout_ms.is_some() || args.stderr_output.is_some() {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--wait-for-exit, --wait-timeout-ms, and --stderr-output are supported only for direct `launch thin`",
            ));
        }
        return map_mcp_launch_options(&args.common);
    }
    let _ = target;
    let common = &args.common;
    let external_epf_wait = match (
        args.wait_for_exit,
        args.wait_timeout_ms,
        &args.stderr_output,
    ) {
        (false, None, None) => None,
        (false, _, _) => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--wait-timeout-ms and --stderr-output require --wait-for-exit",
            ));
        }
        (true, Some(timeout_ms), Some(stderr_output)) => {
            if timeout_ms == 0 {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "--wait-timeout-ms must be greater than or equal to 1",
                ));
            }
            Some(ExternalEpfWaitOptions {
                timeout_ms,
                stderr_output: stderr_output.clone(),
            })
        }
        (true, None, _) => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--wait-for-exit requires --wait-timeout-ms",
            ));
        }
        (true, _, None) => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--wait-for-exit requires --stderr-output",
            ));
        }
    };
    Ok(LaunchOptions {
        c: common.c.clone(),
        execute: common.execute.clone(),
        use_privileged_mode: common.use_privileged_mode,
        out: common.output.clone(),
        internal_out: None,
        raw_args: common.raw_keys.clone(),
        external_epf_wait,
    })
}

fn map_mcp_launch_options(args: &LaunchOptionsArgs) -> Result<LaunchOptions, UseCaseError> {
    if args.c.is_some() || args.execute.is_some() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "launch mcp manages /C internally and does not support --c or --execute",
        ));
    }
    if args
        .raw_keys
        .iter()
        .any(|raw| is_reserved_raw_launch_key(raw))
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "launch mcp manages /C and /Execute internally and does not support raw /C or /Execute keys",
        ));
    }

    Ok(LaunchOptions {
        c: None,
        execute: None,
        use_privileged_mode: args.use_privileged_mode,
        out: args.output.clone(),
        internal_out: None,
        raw_args: args.raw_keys.clone(),
        external_epf_wait: None,
    })
}

fn map_mcp_options(args: &LaunchArgs) -> Result<ClientMcpOptionsRequest, UseCaseError> {
    if args
        .mcp_config
        .as_deref()
        .is_some_and(|path| path.contains(';'))
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "--mcp-config must not contain ';' because the /C runMcp payload is semicolon-delimited",
        ));
    }
    if args.mcp_port == Some(0) {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "--mcp-port must be greater than or equal to 1",
        ));
    }
    let addon = match args.mcp_scenario.as_deref() {
        Some("va") => Some(ClientMcpAddonRequest::VanessaAutomation),
        Some(other) => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("unsupported launch mcp scenario: {other}"),
            ));
        }
        None => None,
    };
    Ok(ClientMcpOptionsRequest {
        config_path: args.mcp_config.clone(),
        port: args.mcp_port,
        addon,
        wait_ready: args.wait_ready,
    })
}

/// `--via` уже ограничен clap до двух значений; отказ остаётся на случай,
/// когда адаптер вызывают не из clap.
fn map_launch_via(value: Option<&str>) -> Result<Option<LaunchVia>, UseCaseError> {
    let Some(value) = value else {
        return Ok(None);
    };
    LaunchVia::parse(value).map(Some).ok_or_else(|| {
        UseCaseError::new(
            UseCaseErrorKind::Validation,
            "--via accepts only `web` or `connection`",
        )
    })
}

fn map_mcp_client_mode(mode: Option<&str>) -> Result<ClientMcpMode, UseCaseError> {
    Ok(match mode.unwrap_or("thin") {
        "thin" => ClientMcpMode::Thin,
        "thick" => ClientMcpMode::Thick,
        "ordinary" => ClientMcpMode::Ordinary,
        other => {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("unsupported launch mcp mode: {other}"),
            ));
        }
    })
}

fn is_reserved_raw_launch_key(raw: &str) -> bool {
    ["c", "execute", "out"]
        .iter()
        .any(|key| launch_key_alias_matches(raw, key))
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct LoadJsonData<'a> {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,

    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    pub provider_dispatched: bool,
    pub mode: LoadMode,
    pub artifact_path: &'a Path,
    pub artifact_type: ArtifactBuildMode,
    pub target_kind: LoadTargetKind,
    pub compatibility_state: CompatibilityState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub execution: &'a ExecutionOutcome<LoadExecutionMetadata>,
}

impl<'a> LoadJsonData<'a> {
    fn from_result(result: &'a LoadResult) -> Self {
        let metadata = load_metadata(result);
        Self {
            provider: result.provider.clone(),
            ok: result.execution.is_ok(),
            provider_dispatched: result.provider_dispatched,
            mode: result.mode,
            artifact_path: result.artifact_path.as_path(),
            artifact_type: result.artifact_type,
            target_kind: metadata
                .map(|metadata| metadata.target_kind)
                .unwrap_or(LoadTargetKind::Unknown),
            compatibility_state: metadata
                .map(|metadata| metadata.compatibility_state)
                .unwrap_or(CompatibilityState::NotProbed),
            extension: result.extension.as_deref(),
            platform_log_path: platform_log_path_from_artifacts(&result.execution.artifacts),
            duration_ms: result.duration_ms,
            message: load_message(result),
            execution: &result.execution,
        }
    }
}

fn load_metadata(result: &LoadResult) -> Option<&LoadExecutionMetadata> {
    result.execution.payload.as_ref()
}

fn load_message(result: &LoadResult) -> Option<String> {
    if !result.execution.is_ok() {
        return execution_message(&result.execution);
    }

    let metadata = load_metadata(result)?;
    let mode = match result.mode {
        LoadMode::Load => "load",
        LoadMode::Merge => "combine",
        LoadMode::Update => "update",
    };
    // A preview applied nothing, so it must not claim a successful apply; its own
    // diagnostics below say what it would have done.
    let mut message = if result.provider_dispatched {
        format!(
            "{mode} {} applied successfully after {:?} compatibility probe",
            result.artifact_path.display(),
            metadata.compatibility_state
        )
    } else {
        format!(
            "{mode} {} previewed; nothing applied",
            result.artifact_path.display()
        )
    };
    if !result.execution.diagnostics.is_empty() {
        message.push_str("; ");
        message.push_str(&result.execution.diagnostics.join("; "));
    }
    Some(message)
}

fn build_load_envelope(result: &LoadResult) -> Envelope<LoadJsonData<'_>> {
    Envelope {
        ok: result.execution.is_ok(),
        command: CommandName::Load.as_str().to_owned(),
        duration_ms: result.duration_ms,
        warnings: Vec::new(),
        steps: Vec::new(),
        error: None,
        data: LoadJsonData::from_result(result),
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ArtifactsJsonData<'a> {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,

    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    pub provider_dispatched: bool,
    pub mode: ArtifactBuildMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<&'a str>,
    pub output_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "ArtifactSet::is_empty")]
    pub artifacts: ArtifactSet,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub execution: &'a ExecutionOutcome<ArtifactBuildMetadata>,
}

impl<'a> ArtifactsJsonData<'a> {
    fn from_result(result: &'a ArtifactsResult) -> Self {
        Self {
            provider: result.provider.clone(),
            ok: result.execution.is_ok(),
            provider_dispatched: result.provider_dispatched,
            mode: result.mode,
            source_set: result.source_set.as_deref(),
            extension: result.extension.as_deref(),
            output_path: result
                .execution
                .payload
                .as_ref()
                .map(|metadata| metadata.output_path.clone())
                .unwrap_or_default(),
            platform_log_path: platform_log_path_from_artifacts(&result.execution.artifacts),
            artifacts: artifact_set_from_execution(&result.execution),
            duration_ms: result.duration_ms,
            message: execution_message(&result.execution),
            execution: &result.execution,
        }
    }
}

fn build_artifacts_envelope(result: &ArtifactsResult) -> Envelope<ArtifactsJsonData<'_>> {
    Envelope {
        ok: result.execution.is_ok(),
        command: CommandName::Artifacts.as_str().to_owned(),
        duration_ms: result.duration_ms,
        warnings: Vec::new(),
        steps: Vec::new(),
        error: None,
        data: ArtifactsJsonData::from_result(result),
    }
}

fn execution_message<T>(execution: &ExecutionOutcome<T>) -> Option<String> {
    execution
        .errors
        .first()
        .map(|error| error.message.clone())
        .or_else(|| execution.diagnostics.first().cloned())
}

fn artifact_set_from_execution<T>(execution: &ExecutionOutcome<T>) -> ArtifactSet {
    execution.artifacts.clone().unwrap_or_default()
}

fn platform_log_path_from_artifacts(artifacts: &Option<ArtifactSet>) -> Option<PathBuf> {
    artifacts
        .as_ref()
        .and_then(|artifacts| artifacts.get_by_role(ARTIFACT_ROLE_PLATFORM_LOG))
        .map(Path::to_path_buf)
}

fn test_retained_paths_from_execution(
    execution: &ExecutionOutcome<TestReport>,
) -> Option<RetainedPaths> {
    execution
        .artifacts
        .as_ref()
        .and_then(RetainedPaths::from_artifact_set)
}

fn test_report(result: &TestRunResult) -> Option<&TestReport> {
    result.execution.payload.as_ref()
}

fn render_build_text(result: &BuildResult, presenter: &Presenter, succeeded: bool) {
    // «Без изменений» — не стандартный исход, у него своя подпись.
    let summary = if succeeded
        && result
            .steps
            .iter()
            .all(|step| matches!(step.mode, BuildMode::Skipped) && step.ok)
    {
        TimelineItem::new(TimelineStatus::Succeeded, "Build completed: no changes")
    } else {
        TimelineItem::outcome(timeline_status(succeeded), "Build")
    };
    let receipt = provider_receipt_details(result.provider.as_ref());
    let summary = if receipt.is_empty() {
        summary
    } else {
        summary.with_detail(receipt.join("\n"))
    };
    presenter.print_timeline(&[summary]);
}

fn render_tools_download_text(result: &ToolsDownloadResult, presenter: &Presenter) {
    let mut details = vec![
        format!("tool: {}", result.tool),
        format!("mode: {}", result.mode),
        format!("config: {}", result.config_path.display()),
        format!("local config: {}", result.local_config_path.display()),
    ];
    for destination in &result.destinations {
        details.push(format!(
            "{} {} -> {} ({})",
            destination.tool,
            destination.tag,
            destination.path.display(),
            destination.config
        ));
    }

    single_timeline(
        presenter,
        TimelineStatus::Succeeded,
        "Tools downloaded successfully",
        details,
    );
}

fn timeline_status(ok: bool) -> TimelineStatus {
    if ok {
        TimelineStatus::Succeeded
    } else {
        TimelineStatus::Failed
    }
}

fn timeline_item_with_details(
    status: TimelineStatus,
    label: impl Into<String>,
    details: Vec<String>,
) -> TimelineItem {
    let item = TimelineItem::new(status, label);
    if details.is_empty() {
        item
    } else {
        item.with_detail(details.join("\n"))
    }
}

fn step_status_detail(status: &InitStepStatus, message: impl AsRef<str>) -> String {
    match status {
        InitStepStatus::Ok => format!("✓ {}", message.as_ref()),
        InitStepStatus::Skipped => format!("○ {}", message.as_ref()),
        InitStepStatus::Failed => format!("✗ {}", message.as_ref()),
        InitStepStatus::Planned => format!("→ {}", message.as_ref()),
    }
}

fn bracketed_detail(kind: &str, message: impl AsRef<str>) -> String {
    format!("[{kind}] {}", message.as_ref())
}

fn timeline_outcome_with_details(
    status: TimelineStatus,
    subject: impl Into<String>,
    details: Vec<String>,
) -> TimelineItem {
    let item = TimelineItem::outcome(status, subject);
    if details.is_empty() {
        item
    } else {
        item.with_detail(details.join("\n"))
    }
}

fn single_timeline(
    presenter: &Presenter,
    status: TimelineStatus,
    label: impl Into<String>,
    details: Vec<String>,
) {
    presenter.print_timeline(&[timeline_item_with_details(status, label, details)]);
}

/// Узел со стандартным исходом: рендерер называет предмет, слово выбирает presenter
/// тем же правилом, что и знак.
fn single_timeline_outcome(
    presenter: &Presenter,
    status: TimelineStatus,
    subject: impl Into<String>,
    details: Vec<String>,
) {
    let item = TimelineItem::outcome(status, subject);
    let item = if details.is_empty() {
        item
    } else {
        item.with_detail(details.join("\n"))
    };
    presenter.print_timeline(&[item]);
}

fn append_if_present(details: &mut Vec<String>, line: Option<String>) {
    if let Some(line) = line.filter(|value| !value.is_empty()) {
        push_unique_detail(details, line);
    }
}

fn push_unique_detail(details: &mut Vec<String>, line: impl Into<String>) {
    let line = line.into();
    if !details.contains(&line) {
        details.push(line);
    }
}

fn append_error_details(details: &mut Vec<String>, errors: &[ExecutionError]) {
    for error in errors {
        push_unique_detail(
            details,
            bracketed_detail(&format!("error:{}", error.code), &error.message),
        );
        for detail in &error.details {
            push_unique_detail(details, bracketed_detail("detail", detail));
        }
        if let Some(artifact) = error.artifact.as_ref() {
            push_unique_detail(details, render_artifact_ref("diagnostic", artifact));
        }
    }
}

fn append_diagnostics(details: &mut Vec<String>, diagnostics: &[String]) {
    for diagnostic in diagnostics {
        push_unique_detail(details, bracketed_detail("diagnostic", diagnostic));
    }
}

fn append_interruptions(details: &mut Vec<String>, interruptions: &[ExecutionInterruptionDetails]) {
    for interruption in interruptions {
        if let Some(message) = interruption.message.as_deref() {
            push_unique_detail(details, bracketed_detail("warning", message));
            continue;
        }

        let kind = match interruption.kind {
            crate::domain::execution::ExecutionInterruptionKind::Cancelled => "cancelled",
            crate::domain::execution::ExecutionInterruptionKind::TimedOut => "timed_out",
        };
        let phase = interruption.phase.as_deref().unwrap_or("unknown_phase");
        let detail = if interruption.deferred {
            format!("deferred {kind} interruption during {phase}")
        } else {
            format!("{kind} interruption during {phase}")
        };
        push_unique_detail(details, bracketed_detail("warning", detail));
    }
}

fn render_artifact_ref(kind: &str, artifact: &ArtifactRef) -> String {
    let role = artifact.role.as_deref().unwrap_or(kind);
    format!("[{kind}] {role} -> {}", artifact.path.display())
}

fn render_output_artifact(path: &Path) -> String {
    format!("[artifact] {}", path.display())
}

fn render_step_signal(step: &StepResult) -> String {
    let label = render_test_step_label(&step.name);
    let message = step
        .message
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("completed");
    match step.status {
        ExecutionStepStatus::Failed => format!("✗ {label}: {message}"),
        ExecutionStepStatus::Skipped => format!("○ {label}: {message}"),
        ExecutionStepStatus::Degraded => {
            bracketed_detail("step:degraded", format!("{label}: {message}"))
        }
        ExecutionStepStatus::Succeeded => format!("✓ {label}: {message}"),
    }
}

fn append_step_signals(details: &mut Vec<String>, steps: &[StepResult]) {
    for step in steps {
        let is_interesting = !matches!(step.status, ExecutionStepStatus::Succeeded)
            || !step.diagnostics.is_empty()
            || !step.errors.is_empty()
            || step.artifacts.is_some();
        if !is_interesting {
            continue;
        }

        push_unique_detail(details, render_step_signal(step));
        if let Some(target) = step.target.as_deref() {
            push_unique_detail(details, bracketed_detail("target", target));
        }
        append_diagnostics(details, &step.diagnostics);
        append_error_details(details, &step.errors);
        if let Some(artifacts) = step.artifacts.as_ref() {
            for artifact in &artifacts.items {
                push_unique_detail(details, render_artifact_ref("artifact", artifact));
            }
        }
    }
}

fn append_report_failures(details: &mut Vec<String>, result: &TestRunResult) {
    let Some(report) = test_report(result) else {
        return;
    };

    for extracted in &report.extracted_errors {
        push_unique_detail(details, bracketed_detail("error:test_report", extracted));
    }

    for suite in &report.suites {
        for case in &suite.cases {
            if matches!(case.status, TestStatus::Passed) {
                continue;
            }

            push_unique_detail(
                details,
                bracketed_detail(
                    "case",
                    format!(
                        "{} :: {} {}",
                        suite.name,
                        status_label(&case.status),
                        case.name
                    ),
                ),
            );
            if let Some(message) = case.failure_message.as_deref() {
                push_unique_detail(details, bracketed_detail("detail", message));
            }
            if let Some(trace) = case.stack_trace.as_deref() {
                push_unique_detail(details, bracketed_detail("detail", trace));
            }
        }
    }
}

fn append_retained_test_artifacts(details: &mut Vec<String>, result: &TestRunResult) {
    let Some(paths) = test_retained_paths_from_execution(&result.execution) else {
        return;
    };

    push_unique_detail(
        details,
        format!("[artifact] run_dir -> {}", paths.run_dir.display()),
    );
    push_unique_detail(
        details,
        format!("[artifact] report -> {}", paths.junit_xml.display()),
    );
    push_unique_detail(
        details,
        format!("[artifact] runner_log -> {}", paths.yaxunit_log.display()),
    );
    push_unique_detail(
        details,
        format!(
            "[diagnostic] platform_log -> {}",
            paths.platform_log.display()
        ),
    );
}

fn should_hide_success_test_diagnostic(diagnostic: &str) -> bool {
    diagnostic.trim_start().starts_with("platform ")
}

fn visible_test_diagnostics(result: &TestRunResult) -> Vec<String> {
    if !result.execution.is_ok() {
        return result.execution.diagnostics.clone();
    }

    result
        .execution
        .diagnostics
        .iter()
        .filter(|diagnostic| !should_hide_success_test_diagnostic(diagnostic))
        .cloned()
        .collect()
}

fn test_has_actionable_success_signal(result: &TestRunResult) -> bool {
    test_report(result).is_some_and(|report| !report.extracted_errors.is_empty())
        || !visible_test_diagnostics(result).is_empty()
}

fn dump_has_warning(result: &DumpResult) -> bool {
    !result.up_to_date
        && result
            .message
            .as_deref()
            .is_some_and(|message| message != crate::domain::dump::DUMP_SUCCESS_MESSAGE)
}

fn execution_has_warning(
    diagnostics: &[String],
    interruptions: &[ExecutionInterruptionDetails],
) -> bool {
    !diagnostics.is_empty() || !interruptions.is_empty()
}

fn render_artifact_mode(mode: ArtifactBuildMode) -> &'static str {
    match mode {
        ArtifactBuildMode::Unknown => "unknown",
        ArtifactBuildMode::ConfigurationCf => "cf",
        ArtifactBuildMode::ExtensionCfe => "cfe",
        ArtifactBuildMode::ExternalDataProcessorEpf => "epf",
        ArtifactBuildMode::ExternalReportErf => "erf",
    }
}

fn render_load_text(result: &LoadResult, presenter: &Presenter, succeeded: bool) {
    let mode = match result.mode {
        LoadMode::Load => "load",
        LoadMode::Merge => "combine",
        LoadMode::Update => "update",
    };
    let metadata = load_metadata(result);
    let target = match metadata
        .map(|metadata| metadata.target_kind)
        .unwrap_or(LoadTargetKind::Unknown)
    {
        crate::domain::load::LoadTargetKind::Configuration => "configuration".to_owned(),
        crate::domain::load::LoadTargetKind::Extension => format!(
            "extension {}",
            result.extension.as_deref().unwrap_or("<unknown>")
        ),
        crate::domain::load::LoadTargetKind::Unknown => "unknown".to_owned(),
    };
    // Рендерер решает, какие подробности показать; слово исхода выбирает presenter.
    let show_signals = !succeeded
        || execution_has_warning(
            &result.execution.diagnostics,
            &result.execution.interruptions,
        );
    let mut details = vec![
        format!("target: {target}"),
        format!(
            "action: {mode} {}",
            render_artifact_mode(result.artifact_type)
        ),
        format!("artifact: {}", result.artifact_path.display()),
    ];
    if show_signals {
        let prefix = if succeeded { "warning" } else { "error" };
        append_if_present(
            &mut details,
            load_message(result).map(|message| bracketed_detail(prefix, message)),
        );
        append_error_details(&mut details, &result.execution.errors);
        append_diagnostics(&mut details, &result.execution.diagnostics);
        append_interruptions(&mut details, &result.execution.interruptions);
        append_if_present(
            &mut details,
            platform_log_path_from_artifacts(&result.execution.artifacts)
                .as_deref()
                .map(|path| format!("[diagnostic] platform log -> {}", path.display())),
        );
    }
    details.extend(provider_receipt_details(result.provider.as_ref()));
    single_timeline_outcome(
        presenter,
        timeline_status(succeeded),
        "Artifact load",
        details,
    );
}

fn render_init_text(result: &InitResult, presenter: &Presenter) {
    let mut details = Vec::new();
    for step in &result.steps {
        if is_designer_edt_workspace_noop(step) {
            continue;
        }

        let line = format!(
            "{}: {} - {}",
            step.target,
            step.action,
            step.message.as_deref().unwrap_or("ok")
        );
        details.push(step_status_detail(&step.status, line));
    }

    let succeeded = result
        .steps
        .iter()
        // Превью ничего не делает и потому ничего не проваливает: шаг со статусом
        // `Planned` — это план, а не отказ. Раньше он приводил к подписи «Init failed»
        // при `ok: true` и коде выхода 0, то есть текст говорил обратное всему остальному.
        .all(|step| {
            matches!(
                step.status,
                InitStepStatus::Ok | InitStepStatus::Skipped | InitStepStatus::Planned
            )
        });
    let mut timeline = vec![timeline_item_with_details(
        timeline_status(succeeded),
        "init:",
        details,
    )];
    timeline.push(timeline_outcome_with_details(
        timeline_status(succeeded),
        "Init",
        provider_receipt_details(result.provider.as_ref()),
    ));
    presenter.print_timeline(&timeline);
}

fn is_designer_edt_workspace_noop(step: &InitStep) -> bool {
    matches!(step.status, InitStepStatus::Skipped)
        && step.target == "edt_workspace"
        && step.action == "import"
        && step
            .message
            .as_deref()
            .is_some_and(|message| message.contains("format=DESIGNER"))
}

fn render_dump_text(result: &DumpResult, presenter: &Presenter, succeeded: bool) {
    let mode = match result.mode {
        DumpMode::Full => "full",
        DumpMode::Incremental => "incremental",
        DumpMode::Partial => "partial",
    };
    let source_set = result.source_set.as_deref().unwrap_or("<unresolved>");
    let show_signals = !succeeded || dump_has_warning(result);
    let skipped_label =
        (succeeded && result.up_to_date).then_some("Dump skipped: configuration unchanged");
    let mut details = vec![
        format!("source-set: {source_set}"),
        format!("mode: {mode}"),
        format!("output: {}", result.target_path.display()),
    ];
    if let Some(extension) = result.extension.as_deref() {
        details.push(format!("extension: {extension}"));
    }
    if succeeded && result.up_to_date {
        append_if_present(
            &mut details,
            result
                .message
                .as_deref()
                .map(|message| bracketed_detail("note", message)),
        );
    }
    if show_signals {
        let prefix = if succeeded { "warning" } else { "error" };
        append_if_present(
            &mut details,
            result
                .message
                .as_deref()
                .map(|message| bracketed_detail(prefix, message)),
        );
        append_if_present(
            &mut details,
            result
                .platform_log_path
                .as_deref()
                .map(|path| format!("[diagnostic] platform log -> {}", path.display())),
        );
    }
    details.extend(provider_receipt_details(result.provider.as_ref()));
    // «Пропущено» — не стандартный исход, у него своя подпись; в остальных случаях
    // слово выбирает presenter.
    match skipped_label {
        Some(label) => single_timeline(presenter, timeline_status(succeeded), label, details),
        None => single_timeline_outcome(presenter, timeline_status(succeeded), "Dump", details),
    }
}

fn render_convert_text(result: &ConvertResult, presenter: &Presenter, succeeded: bool) {
    let mut details = vec![
        format!("direction: {}", render_convert_direction(result.direction)),
        format!(
            "scope: {}",
            render_convert_scope(result.scope, result.source_set.as_deref())
        ),
        format!("workspace: {}", result.workspace_path.display()),
    ];
    for output in &result.outputs {
        details.push(format!(
            "source-set {}: {} -> {}",
            output.source_set,
            output.source_path.display(),
            output.target_path.display()
        ));
    }
    if !succeeded || result.message.is_some() {
        let prefix = if succeeded { "warning" } else { "error" };
        append_if_present(
            &mut details,
            result
                .message
                .as_deref()
                .map(|message| bracketed_detail(prefix, message)),
        );
    }
    single_timeline_outcome(presenter, timeline_status(succeeded), "Convert", details);
}

fn render_artifacts_text(result: &ArtifactsResult, presenter: &Presenter, succeeded: bool) {
    let source_set = result.source_set.as_deref().unwrap_or("<unresolved>");
    let message = execution_message(&result.execution);
    // Рендерер решает, какие подробности показать; слово исхода выбирает presenter.
    let show_signals = !succeeded
        || message.is_some()
        || execution_has_warning(
            &result.execution.diagnostics,
            &result.execution.interruptions,
        );
    let mut details = vec![
        format!("source-set: {source_set}"),
        format!("mode: {}", render_artifact_mode(result.mode)),
        format!(
            "output: {}",
            result
                .execution
                .payload
                .as_ref()
                .map(|metadata| metadata.output_path.display().to_string())
                .unwrap_or_else(|| "<unresolved>".to_owned())
        ),
    ];
    if let Some(extension) = result.extension.as_deref() {
        details.push(format!("extension: {extension}"));
    }
    let package_artifacts = result
        .execution
        .artifacts
        .as_ref()
        .into_iter()
        .flat_map(|artifacts| artifacts.items.iter())
        .filter(|artifact| artifact.role.as_deref() == Some(ARTIFACT_ROLE_PACKAGE_FILE))
        .collect::<Vec<_>>();
    if package_artifacts.is_empty() {
        if let Some(metadata) = result.execution.payload.as_ref() {
            details.push(render_output_artifact(&metadata.output_path));
        }
    } else {
        for artifact in package_artifacts {
            details.push(render_artifact_ref("artifact", artifact));
        }
    }
    if show_signals {
        let prefix = if succeeded { "warning" } else { "error" };
        append_if_present(
            &mut details,
            message.map(|message| bracketed_detail(prefix, message)),
        );
        append_error_details(&mut details, &result.execution.errors);
        append_diagnostics(&mut details, &result.execution.diagnostics);
        append_interruptions(&mut details, &result.execution.interruptions);
        append_if_present(
            &mut details,
            platform_log_path_from_artifacts(&result.execution.artifacts)
                .as_deref()
                .map(|path| format!("[diagnostic] platform log -> {}", path.display())),
        );
        for artifact in result
            .execution
            .artifacts
            .as_ref()
            .into_iter()
            .flat_map(|artifacts| artifacts.items.iter())
            .filter(|artifact| artifact.role.as_deref() == Some(ARTIFACT_ROLE_PLATFORM_LOG))
        {
            details.push(render_artifact_ref("diagnostic", artifact));
        }
    }
    details.extend(provider_receipt_details(result.provider.as_ref()));
    single_timeline_outcome(
        presenter,
        timeline_status(succeeded),
        "Artifacts export",
        details,
    );
}

fn render_convert_direction(direction: ConvertDirection) -> &'static str {
    match direction {
        ConvertDirection::EdtToDesigner => "edt-to-designer",
        ConvertDirection::DesignerToEdt => "designer-to-edt",
    }
}

fn render_convert_scope(scope: ConvertScope, source_set: Option<&str>) -> String {
    match (scope, source_set) {
        (ConvertScope::All, _) => "all source-sets".to_owned(),
        (ConvertScope::Single, Some(source_set)) => format!("source-set {source_set}"),
        (ConvertScope::Single, None) => "single source-set".to_owned(),
    }
}

fn render_syntax_text(result: &SyntaxCheckResult, presenter: &Presenter) {
    // Превью — исход успешный: проверка не выполнялась, значит и приговора нет.
    let succeeded = matches!(
        result.status,
        SyntaxCheckStatus::Clean | SyntaxCheckStatus::Planned
    );
    // «Найдены замечания» — не стандартный исход, у него своя подпись. У остальных
    // слово выбирает presenter. Непрочитанный журнал больше не остаётся одним
    // предупреждением среди подробностей: он делает вердикт неизвестным, то есть
    // `tool_failed`, и подпись следует за знаком сама.
    let subject = if result.provider_dispatched {
        format!("Syntax check {}", result.check_name)
    } else {
        format!("Syntax check {} preview", result.check_name)
    };
    let issues_label = matches!(result.status, SyntaxCheckStatus::IssuesFound)
        .then(|| format!("{subject} found issues"));
    let mut details = vec![format!(
        "status: {} (exit {}, errors {}, warnings {}, info {}, duration {} ms)",
        render_syntax_status(result.status),
        result.exit_code,
        result.summary.errors,
        result.summary.warnings,
        result.summary.info,
        result.duration_ms
    )];

    append_if_present(
        &mut details,
        result
            .message
            .as_deref()
            .map(|message| bracketed_detail("status", message)),
    );
    if !result.provider_dispatched {
        details.push("provider dispatched: false".to_owned());
    }

    if !succeeded {
        for issue in &result.issues {
            details.push(bracketed_detail("issue", render_issue(issue)));
        }
    }

    append_if_present(
        &mut details,
        result
            .log_read_warning
            .as_deref()
            .map(|warning| bracketed_detail("warning", format!("log {warning}"))),
    );

    if !succeeded || result.log_read_warning.is_some() {
        append_if_present(
            &mut details,
            result
                .platform_log_path
                .as_deref()
                .map(|path| format!("[diagnostic] platform log -> {}", path.display())),
        );
    }

    if matches!(result.status, SyntaxCheckStatus::ToolFailed) {
        append_if_present(
            &mut details,
            result
                .stderr
                .as_deref()
                .map(|stderr| bracketed_detail("diagnostic", format!("stderr: {}", stderr.trim()))),
        );
    }

    details.extend(provider_receipt_details(result.provider.as_ref()));
    match issues_label {
        Some(label) => single_timeline(presenter, timeline_status(succeeded), label, details),
        None => single_timeline_outcome(presenter, timeline_status(succeeded), subject, details),
    }
}

fn render_syntax_status(status: SyntaxCheckStatus) -> &'static str {
    match status {
        SyntaxCheckStatus::Clean => "clean",
        SyntaxCheckStatus::IssuesFound => "issues_found",
        SyntaxCheckStatus::ToolFailed => "tool_failed",
        SyntaxCheckStatus::Planned => "planned",
    }
}

fn render_launch_text(result: &LaunchResult, presenter: &Presenter) {
    let subject = if result.provider_dispatched {
        "Launch"
    } else {
        "Launch preview"
    };
    render_launch_text_with_status(result, presenter, TimelineStatus::Succeeded, subject);
}

fn render_launch_text_with_status(
    result: &LaunchResult,
    presenter: &Presenter,
    status: TimelineStatus,
    subject: &'static str,
) {
    let mut details = vec![
        format!("mode: {}", render_launch_mode(&result.mode)),
        format!("binary: {}", result.binary.display()),
    ];
    if let Some(url) = result.url.as_deref() {
        details.push(format!("url: {url}"));
    }
    append_if_present(
        &mut details,
        result
            .message
            .as_deref()
            .map(|message| bracketed_detail("status", message)),
    );
    if let Some(pid) = result.pid {
        details.push(format!("pid: {pid}"));
    }
    if !result.provider_dispatched {
        details.push("provider dispatched: false".to_owned());
    }
    if let Some(plan) = &result.plan {
        details.push(format!("planned program: {}", plan.program.display()));
        details.push(format!("planned args: {}", plan.args.join(" ")));
    }
    if let Some(readiness) = &result.mcp_readiness {
        details.push(format!("mcp endpoint: {}", readiness.url));
        details.push(format!(
            "mcp ready: {}",
            if readiness.ok { "yes" } else { "no" }
        ));
        if !readiness.tools.is_empty() {
            details.push(format!("mcp tools: {}", readiness.tools.join(", ")));
        }
        if !readiness.missing_tools.is_empty() {
            details.push(format!(
                "missing mcp tools: {}",
                readiness.missing_tools.join(", ")
            ));
        }
    }
    single_timeline_outcome(presenter, status, subject, details);
}

fn render_launch_mode(mode: &LaunchMode) -> &'static str {
    match mode {
        LaunchMode::Designer => "конфигуратор",
        LaunchMode::Thin => "тонкий клиент",
        LaunchMode::Thick => "толстый клиент",
        LaunchMode::Ordinary => "обычное приложение",
        LaunchMode::Mcp => "клиентский MCP-сервер",
        LaunchMode::Web => "веб-клиент",
    }
}

fn render_test_text(result: &TestRunResult, presenter: &Presenter) {
    let diagnostics = visible_test_diagnostics(result);
    let succeeded = result.execution.is_ok();
    // Рендерер решает, показывать ли сигналы шагов; слово исхода выбирает presenter.
    let show_signals = !succeeded
        || !result.warnings.is_empty()
        || test_has_actionable_success_signal(result)
        || !result.execution.interruptions.is_empty()
        || result
            .steps
            .iter()
            .any(|step| !matches!(step.status, ExecutionStepStatus::Succeeded));
    let mut details = vec![format!("target: {}", render_test_target(&result.target))];
    if let Some(report) = test_report(result) {
        details.push(format!(
            "summary: total={}, passed={}, failed={}, skipped={}, errors={}",
            report.summary.total,
            report.summary.passed,
            report.summary.failed,
            report.summary.skipped,
            report.summary.errors
        ));
    }

    if show_signals {
        append_step_signals(&mut details, &result.steps);
        append_report_failures(&mut details, result);
        append_error_details(&mut details, &result.execution.errors);
        append_diagnostics(&mut details, &diagnostics);
        append_interruptions(&mut details, &result.execution.interruptions);
        for warning in &result.warnings {
            push_unique_detail(&mut details, bracketed_detail("warning", warning));
        }
        append_retained_test_artifacts(&mut details, result);
    }

    single_timeline_outcome(presenter, timeline_status(succeeded), "Tests", details);
}

fn render_test_target(target: &TestTarget) -> String {
    match target {
        TestTarget::All => "all".to_owned(),
        TestTarget::Module { name } => format!("module {name}"),
    }
}

fn render_test_step_label(name: &str) -> String {
    match name {
        "build" => "build prerequisite".to_owned(),
        "prepare_artifacts" => "prepare artifacts".to_owned(),
        "prepare_runner" => "prepare runner".to_owned(),
        "run" => "enterprise run".to_owned(),
        "parse_junit" => "parse JUnit report".to_owned(),
        "parse_log" => "parse runner log".to_owned(),
        other => other.to_owned(),
    }
}

fn render_issue(issue: &Issue) -> String {
    match issue {
        Issue::Module(issue) => {
            let location = match (issue.line, issue.column) {
                (Some(line), Some(column)) => format!("{}:{}:{}", issue.path, line, column),
                (Some(line), None) => format!("{}:{}", issue.path, line),
                _ => issue.path.clone(),
            };
            format!(
                "{} {} {}",
                render_severity(&issue.severity),
                location,
                issue.message
            )
        }
        Issue::Object(issue) => format!(
            "{} {} {}",
            render_severity(&issue.severity),
            issue.object,
            issue.message
        ),
        Issue::Edt(issue) => {
            let location = match (issue.line, issue.column) {
                (Some(line), Some(column)) => format!("{}:{}:{}", issue.path, line, column),
                (Some(line), None) => format!("{}:{}", issue.path, line),
                _ => issue.path.clone(),
            };
            format!(
                "{} {} {}",
                render_severity(&issue.severity),
                location,
                issue.message
            )
        }
    }
}

fn render_severity(severity: &IssueSeverity) -> &'static str {
    match severity {
        IssueSeverity::Error => "ERROR",
        IssueSeverity::Warning => "WARNING",
        IssueSeverity::Info => "INFO",
    }
}

fn status_label(status: &TestStatus) -> &'static str {
    match status {
        TestStatus::Passed => "PASSED",
        TestStatus::Failed => "FAILED",
        TestStatus::Skipped => "SKIPPED",
        TestStatus::Error => "ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_load_envelope, command_name, execute_command, infobase_pre_dispatch_execution_phase,
        map_artifacts_request_with_config, map_build_request, map_designer_config_request,
        map_dump_request, map_extensions_request, map_launch_request, map_load_request,
        map_syntax_request, map_test_request,
    };
    use crate::cli::args::{
        ArtifactsArgs, BuildArgs, Command, DesignerConfigSyntaxArgs, DesignerModulesSyntaxArgs,
        DirectLaunchOptionsArgs, DumpArgs, ExtensionsArgs, InfobaseArgs, InfobaseCommand,
        InfobaseConfigurationArgs, InfobaseConfigurationCommand, InfobaseConfigurationExportArgs,
        InfobaseDumpArgs, LaunchArgs, LaunchOptionsArgs, LoadArgs, SyntaxArgs, SyntaxTarget,
        TestArgs, TestLaunchOptionsArgs, TestRunner, TestScope, TestVaArgs, TestYaxunitArgs,
    };
    use crate::cli::output::pre_dispatch_error_envelope;
    use crate::config::model::{
        AppConfig, BuildConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
    };
    use crate::domain::artifacts::ArtifactBuildMode;
    use crate::domain::execution::{ExecutionOutcome, ExecutionStatus};
    use crate::domain::infobase_export::InfobaseTransferPhase;
    use crate::domain::load::{
        CompatibilityState, LoadExecutionMetadata, LoadMode, LoadResult, LoadTargetKind,
    };
    use crate::domain::runner::{LaunchOptions, RunnerKind};
    use crate::output::presenter::{ColorMode, Presenter};
    use crate::support::fs::acquire_advisory_lock;
    use crate::support::temp::platform_logs_dir;
    use crate::use_cases::context::CommandName;
    use crate::use_cases::request::{
        ArtifactsModeRequest, ClientMcpAddonRequest, ClientMcpMode, ClientMcpOptionsRequest,
        DesignerClientScope, DesignerConfigCheck, DumpModeRequest, LaunchRequest,
        LaunchTargetRequest, SyntaxTargetRequest, TestBuildPolicy, TestScopeRequest,
    };
    use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};
    use crate::use_cases::workspace_lock::workspace_lock_path;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn maps_test_module_request() {
        let work = tempdir().expect("tempdir");
        let config = sample_config(work.path());
        let request = map_test_request(
            &config,
            &TestArgs {
                full: true,
                no_build: false,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Yaxunit(TestYaxunitArgs {
                    scope: TestScope::Module {
                        name: "ModuleA".to_owned(),
                    },
                }),
            },
        )
        .expect("request");

        assert!(request.full);
        assert_eq!(request.build_policy, TestBuildPolicy::BuildFirst);
        assert_eq!(
            request.scope,
            TestScopeRequest::Module {
                name: "ModuleA".to_owned()
            }
        );
    }

    #[test]
    fn maps_no_build_yaxunit_request() {
        let work = tempdir().expect("tempdir");
        let config = sample_config(work.path());
        let request = map_test_request(
            &config,
            &TestArgs {
                full: false,
                no_build: true,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Yaxunit(TestYaxunitArgs {
                    scope: TestScope::All,
                }),
            },
        )
        .expect("request");

        assert_eq!(request.build_policy, TestBuildPolicy::Skip);
    }

    #[test]
    fn rejects_blank_test_module_request() {
        let work = tempdir().expect("tempdir");
        let config = sample_config(work.path());
        let error = map_test_request(
            &config,
            &TestArgs {
                full: false,
                no_build: false,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Yaxunit(TestYaxunitArgs {
                    scope: TestScope::Module {
                        name: "   ".to_owned(),
                    },
                }),
            },
        )
        .expect_err("blank module should be rejected");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(
            error.message(),
            "test module requires a non-empty module name"
        );
    }

    #[test]
    fn maps_vanessa_request_from_configured_profile() {
        let work = tempdir().expect("tempdir");
        let base = work.path().join("base");
        let features = work.path().join("features");
        let epf = work.path().join("va.epf");
        let params = work.path().join("va.json");
        std::fs::create_dir_all(base.join("src")).expect("src");
        std::fs::create_dir_all(&features).expect("features");
        std::fs::write(&epf, "epf").expect("epf");
        std::fs::write(&params, "{}").expect("params");

        let mut config = sample_config(work.path());
        config.base_path = base;
        config.tools.va.epf_path = Some(epf);
        config.tests.va.params_path = Some(params);
        config.tests.va.profile = Some("smoke".to_owned());
        config.tests.va.profiles.insert(
            "smoke".to_owned(),
            crate::config::model::VanessaProfileConfig {
                feature_path: Some(features),
                ..Default::default()
            },
        );

        let request = map_test_request(
            &config,
            &TestArgs {
                full: false,
                no_build: false,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Va(TestVaArgs::default()),
            },
        )
        .expect("request");

        assert_eq!(request.execution.profile.kind, RunnerKind::Vanessa);
        assert_eq!(request.execution.profile.id, "smoke");
        assert_eq!(request.build_policy, TestBuildPolicy::BuildFirst);
        assert_eq!(request.scope, TestScopeRequest::All);
        assert_eq!(request.execution.timeouts.total_ms, Some(300_000));

        let no_build_request = map_test_request(
            &config,
            &TestArgs {
                full: false,
                no_build: true,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Va(TestVaArgs::default()),
            },
        )
        .expect("no-build request");

        assert_eq!(no_build_request.build_policy, TestBuildPolicy::Skip);
    }

    /// Формат проекта выбирает ветку и без подкоманды: у EDT это проверка проекта, и
    /// `--project` доезжает до неё.
    #[test]
    fn maps_a_bare_check_to_the_edt_branch_by_format() {
        let work = tempfile::tempdir().expect("tempdir");
        let mut config = sample_config(work.path());
        config.format = SourceFormat::Edt;

        let request = map_syntax_request(
            &config,
            &SyntaxArgs {
                modes: DesignerConfigSyntaxArgs::default(),
                projects: vec!["main".to_owned()],
                target: None,
            },
            false,
        )
        .expect("request");

        assert!(matches!(
            request.target,
            SyntaxTargetRequest::Edt { ref projects } if projects == &["main".to_owned()]
        ));
    }

    /// Прежнее имя `designer-modules` исполняется `/CheckConfig`: режимы доезжают, а
    /// проверок конфигурации в запросе нет.
    #[test]
    fn maps_syntax_request() {
        let work = tempfile::tempdir().expect("tempdir");
        let config = sample_config(work.path());
        let request = map_syntax_request(
            &config,
            &SyntaxArgs {
                modes: DesignerConfigSyntaxArgs::default(),
                projects: Vec::new(),
                target: Some(SyntaxTarget::DesignerModules(DesignerModulesSyntaxArgs {
                    thin_client: true,
                    web_client: false,
                    server: true,
                    external_connection: false,
                    thick_client_ordinary_application: false,
                    mobile_app_client: false,
                    mobile_app_server: false,
                    mobile_client: false,
                    extended_modules_check: true,
                    extension: Some("Ext".to_owned()),
                    all_extensions: false,
                })),
            },
            false,
        )
        .expect("request");

        assert!(matches!(
            request.target,
            SyntaxTargetRequest::DesignerConfig(ref modes)
                if modes.has_client_scope(DesignerClientScope::ThinClient)
                    && modes.has_client_scope(DesignerClientScope::Server)
                    && modes.extension_scope().extension() == Some("Ext")
                    && !modes.has_check(crate::use_cases::request::DesignerConfigCheck::UnreferenceProcedures)
        ));
    }

    #[test]
    fn maps_build_dump_launch_and_load_requests() {
        assert!(
            map_build_request(
                &BuildArgs {
                    full_rebuild: true,
                    source_set: None,
                },
                false,
            )
            .full_rebuild
        );
        assert_eq!(
            map_extensions_request(
                &ExtensionsArgs {
                    command: None,
                    names: vec!["client_mcp".to_owned()],
                    installed_names: vec![],
                },
                false,
            )
            .names,
            vec!["client_mcp"]
        );
        assert_eq!(
            map_dump_request(
                &DumpArgs {
                    discard_uncommitted: false,
                    mode: "incremental".to_owned(),
                    source_set: Some("main".to_owned()),
                    extension: Some("Ext".to_owned()),
                    objects: vec!["Catalog.Item".to_owned()],
                },
                false,
            )
            .expect("request")
            .mode,
            DumpModeRequest::Incremental
        );
        assert_eq!(
            map_dump_request(
                &DumpArgs {
                    discard_uncommitted: false,
                    mode: "incremental".to_owned(),
                    source_set: Some("main".to_owned()),
                    extension: Some("Ext".to_owned()),
                    objects: vec!["Catalog.Item".to_owned()],
                },
                false,
            )
            .expect("request")
            .source_set
            .as_deref(),
            Some("main")
        );
        assert_eq!(
            map_launch_request(
                &LaunchArgs {
                    via: None,
                    target: "thin".to_owned(),
                    mcp_scenario: None,
                    mcp_mode: None,
                    launch: DirectLaunchOptionsArgs {
                        common: LaunchOptionsArgs {
                            c: Some("Command".to_owned()),
                            execute: Some("tool.epf".to_owned()),
                            use_privileged_mode: true,
                            output: Some("launch.log".to_owned()),
                            raw_keys: vec!["/WA-".to_owned(), "/DisplayAllFunctions".to_owned()],
                        },
                        ..DirectLaunchOptionsArgs::default()
                    },
                    mcp_config: None,
                    mcp_port: None,
                    wait_ready: false,
                },
                false,
            )
            .expect("request"),
            LaunchRequest {
                via: None,
                target: LaunchTargetRequest::thin_client(),
                launch: LaunchOptions {
                    c: Some("Command".to_owned()),
                    execute: Some("tool.epf".to_owned()),
                    use_privileged_mode: true,
                    out: Some("launch.log".to_owned()),
                    internal_out: None,
                    raw_args: vec!["/WA-".to_owned(), "/DisplayAllFunctions".to_owned()],
                    external_epf_wait: None,
                },
                client_mcp: None,
                dry_run: false,
            }
        );
        assert_eq!(
            map_launch_request(
                &LaunchArgs {
                    via: None,
                    target: "ordinary".to_owned(),
                    mcp_scenario: None,
                    mcp_mode: None,
                    launch: DirectLaunchOptionsArgs::default(),
                    mcp_config: None,
                    mcp_port: None,
                    wait_ready: false,
                },
                false,
            )
            .expect("request")
            .target,
            LaunchTargetRequest::ordinary_application()
        );
        assert_eq!(
            map_launch_request(
                &LaunchArgs {
                    via: None,
                    target: "thin".to_owned(),
                    mcp_scenario: None,
                    mcp_mode: None,
                    launch: DirectLaunchOptionsArgs::default(),
                    mcp_config: None,
                    mcp_port: None,
                    wait_ready: false,
                },
                false,
            )
            .expect("request")
            .target,
            LaunchTargetRequest::thin_client()
        );
        assert_eq!(
            map_launch_request(
                &LaunchArgs {
                    via: None,
                    target: "mcp".to_owned(),
                    mcp_scenario: Some("va".to_owned()),
                    mcp_mode: Some("ordinary".to_owned()),
                    launch: DirectLaunchOptionsArgs::default(),
                    mcp_config: Some("C:\\tmp\\mcp-conf.json".to_owned()),
                    mcp_port: Some(123),
                    wait_ready: true,
                },
                false,
            )
            .expect("request"),
            LaunchRequest {
                via: None,
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Ordinary),
                launch: LaunchOptions {
                    c: None,
                    execute: None,
                    use_privileged_mode: false,
                    out: None,
                    internal_out: None,
                    raw_args: Vec::new(),
                    external_epf_wait: None,
                },
                client_mcp: Some(ClientMcpOptionsRequest {
                    config_path: Some("C:\\tmp\\mcp-conf.json".to_owned()),
                    port: Some(123),
                    addon: Some(ClientMcpAddonRequest::VanessaAutomation),
                    wait_ready: true,
                }),
                dry_run: false,
            }
        );
        let load = map_load_request(
            &LoadArgs {
                path: "dist/main.cf".to_owned(),
                mode: "merge".to_owned(),
                settings: Some("merge.xml".to_owned()),
                vendor_name: None,
                extension: Some("Ext".to_owned()),
            },
            false,
        )
        .expect("load request");
        assert_eq!(load.mode, LoadMode::Merge);
        assert_eq!(load.artifact_path, "dist/main.cf");
        assert_eq!(load.settings_path.as_deref(), Some("merge.xml"));
        assert_eq!(load.extension.as_deref(), Some("Ext"));
        let artifacts = map_artifacts_request_with_config(
            &sample_config(Path::new("/tmp/work")),
            &ArtifactsArgs {
                output: "dist/ext.cfe".to_owned(),
                source_set: Some("ext-sales".to_owned()),
                extension: Some("SalesAddon".to_owned()),
            },
            false,
        )
        .expect("request");
        assert_eq!(artifacts.mode, ArtifactsModeRequest::ExtensionCfe);
        assert_eq!(artifacts.source_set.as_deref(), Some("ext-sales"));
        assert_eq!(artifacts.extension.as_deref(), Some("SalesAddon"));
    }

    #[test]
    fn maps_artifacts_request_keeps_blank_extension_in_cfe_mode() {
        let artifacts = map_artifacts_request_with_config(
            &sample_config(Path::new("/tmp/work")),
            &ArtifactsArgs {
                output: "dist/main.cf".to_owned(),
                source_set: Some("main".to_owned()),
                extension: Some("   ".to_owned()),
            },
            false,
        )
        .expect("request");

        assert_eq!(artifacts.mode, ArtifactsModeRequest::ExtensionCfe);
        assert_eq!(artifacts.extension.as_deref(), Some("   "));
        assert_eq!(artifacts.source_set.as_deref(), Some("main"));
    }

    #[test]
    fn rejects_invalid_mode_mapping() {
        let dump_error = map_dump_request(
            &DumpArgs {
                discard_uncommitted: false,
                mode: "garbage".to_owned(),
                source_set: None,
                extension: None,
                objects: vec![],
            },
            false,
        )
        .expect_err("dump mode should be rejected");
        let launch_error = map_launch_request(
            &LaunchArgs {
                via: None,
                target: "garbage".to_owned(),
                mcp_scenario: None,
                mcp_mode: None,
                launch: DirectLaunchOptionsArgs::default(),
                mcp_config: None,
                mcp_port: None,
                wait_ready: false,
            },
            false,
        )
        .expect_err("launch mode should be rejected");

        assert_eq!(dump_error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(launch_error.kind(), UseCaseErrorKind::Validation);
    }

    #[test]
    fn rejects_invalid_load_mode_mapping() {
        let error = map_load_request(
            &LoadArgs {
                path: "dist/main.cf".to_owned(),
                mode: "garbage".to_owned(),
                settings: None,
                vendor_name: None,
                extension: None,
            },
            false,
        )
        .expect_err("load mode should be rejected");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(error.message(), "unsupported load mode: garbage");
    }

    #[test]
    fn maps_designer_config_request() {
        let request = map_designer_config_request(&DesignerConfigSyntaxArgs {
            config_log_integrity: true,
            incorrect_references: false,
            thin_client: true,
            web_client: false,
            mobile_client: false,
            server: true,
            external_connection: false,
            external_connection_server: false,
            mobile_app_client: false,
            mobile_app_server: false,
            thick_client_managed_application: false,
            thick_client_server_managed_application: false,
            thick_client_ordinary_application: false,
            thick_client_server_ordinary_application: false,
            mobile_client_digi_sign: false,
            distributive_modules: false,
            unreference_procedures: false,
            handlers_existence: false,
            empty_handlers: false,
            extended_modules_check: true,
            check_use_synchronous_calls: true,
            check_use_modality: false,
            unsupported_functional: false,
            extension: Some("Ext".to_owned()),
            all_extensions: false,
        })
        .expect("request");

        assert!(request.has_check(DesignerConfigCheck::ConfigLogIntegrity));
        assert!(request.has_client_scope(DesignerClientScope::ThinClient));
        assert!(request.has_client_scope(DesignerClientScope::Server));
        assert!(request.extended_modules().is_enabled());
        assert!(request.extended_modules().checks_synchronous_calls());
    }

    #[test]
    fn rejects_invalid_config_extended_modules_mapping() {
        let error = map_designer_config_request(&DesignerConfigSyntaxArgs {
            config_log_integrity: false,
            incorrect_references: false,
            thin_client: false,
            web_client: false,
            mobile_client: false,
            server: false,
            external_connection: false,
            external_connection_server: false,
            mobile_app_client: false,
            mobile_app_server: false,
            thick_client_managed_application: false,
            thick_client_server_managed_application: false,
            thick_client_ordinary_application: false,
            thick_client_server_ordinary_application: false,
            mobile_client_digi_sign: false,
            distributive_modules: false,
            unreference_procedures: false,
            handlers_existence: false,
            empty_handlers: false,
            extended_modules_check: false,
            check_use_synchronous_calls: true,
            check_use_modality: false,
            unsupported_functional: false,
            extension: None,
            all_extensions: false,
        })
        .expect_err("invalid dependency");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(
            error.message(),
            "check-use-synchronous-calls requires extended-modules-check=true"
        );
    }

    #[test]
    fn resolves_command_name() {
        assert_eq!(command_name(&Command::Init), CommandName::Init);
        assert_eq!(
            command_name(&Command::Extensions(ExtensionsArgs {
                command: None,
                names: vec![],
                installed_names: vec![],
            })),
            CommandName::Extensions
        );
        assert_eq!(
            command_name(&Command::Build(BuildArgs {
                full_rebuild: false,
                source_set: None,
            })),
            CommandName::Build
        );
        assert_eq!(
            command_name(&Command::Load(LoadArgs {
                path: "dist/main.cf".to_owned(),
                mode: "load".to_owned(),
                settings: None,
                vendor_name: None,
                extension: None,
            })),
            CommandName::Load
        );
        assert_eq!(
            command_name(&Command::Artifacts(ArtifactsArgs {
                output: "dist/main.cf".to_owned(),
                source_set: None,
                extension: None,
            })),
            CommandName::Artifacts
        );
    }

    fn sample_config(work_path: &Path) -> AppConfig {
        AppConfig {
            base_path: work_path.join("base"),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: PathBuf::from("main"),
                },
                SourceSetConfig {
                    name: "ext-sales".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: PathBuf::from("ext-sales"),
                },
                SourceSetConfig {
                    name: "external-processors".to_owned(),
                    purpose: SourceSetPurpose::ExternalDataProcessors,
                    path: PathBuf::from("external-processors"),
                },
            ],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn execute_command_reports_workspace_lock_conflict_before_dispatch() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);

        let error = execute_command(
            &config,
            &Command::Build(BuildArgs {
                full_rebuild: true,
                source_set: None,
            }),
            None,
            &presenter,
            false,
            false,
        )
        .expect_err("busy workspace");

        assert_eq!(error.kind(), UseCaseErrorKind::Runtime);
        assert!(error.to_string().contains("workspace"));
        assert!(error.to_string().contains("already"));
    }

    #[test]
    fn execute_command_reports_workspace_lock_conflict_for_test_command() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);

        let error = execute_command(
            &config,
            &Command::Test(TestArgs {
                full: false,
                no_build: false,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Yaxunit(TestYaxunitArgs {
                    scope: TestScope::All,
                }),
            }),
            None,
            &presenter,
            false,
            false,
        )
        .expect_err("busy workspace");

        assert_eq!(error.kind(), UseCaseErrorKind::Runtime);
        assert!(error.to_string().contains("workspace"));
        assert!(error.to_string().contains("already"));
    }

    #[test]
    fn infobase_commands_report_workspace_lock_conflict_before_provider_dispatch() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let infobase = dir.path().join("ib");
        fs::create_dir_all(&infobase).expect("infobase dir");
        fs::write(infobase.join("1Cv8.1CD"), b"fixture").expect("infobase file");
        let platform = dir
            .path()
            .join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
        fs::write(&platform, b"fixture").expect("platform executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&platform)
                .expect("platform metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&platform, permissions).expect("platform permissions");
        }
        let mut config = sample_config(&work);
        config.infobase =
            crate::config::model::InfobaseConfig::file(format!("File={}", infobase.display()));
        config.tools.platform.path = Some(platform);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);
        let commands = [
            Command::Infobase(InfobaseArgs {
                command: InfobaseCommand::Configuration(InfobaseConfigurationArgs {
                    command: InfobaseConfigurationCommand::Export(
                        InfobaseConfigurationExportArgs {
                            state: "working".to_owned(),
                            extension: None,
                            output: dir.path().join("main.cf").display().to_string(),
                        },
                    ),
                }),
            }),
            Command::Infobase(InfobaseArgs {
                command: InfobaseCommand::Dump(InfobaseDumpArgs {
                    output: dir.path().join("base.dt").display().to_string(),
                }),
            }),
        ];

        for command in commands {
            let error = execute_command(&config, &command, None, &presenter, false, false)
                .expect_err("busy workspace");
            assert_eq!(error.kind(), UseCaseErrorKind::WorkspaceBusy);
            assert!(error.to_string().contains("workspace"));
            assert!(error.to_string().contains("already"));
        }
    }

    #[test]
    fn execute_command_validates_before_trying_workspace_lock() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);

        let error = execute_command(
            &config,
            &Command::Launch(LaunchArgs {
                via: None,
                target: "garbage".to_owned(),
                mcp_scenario: None,
                mcp_mode: None,
                launch: DirectLaunchOptionsArgs::default(),
                mcp_config: None,
                mcp_port: None,
                wait_ready: false,
            }),
            None,
            &presenter,
            false,
            false,
        )
        .expect_err("invalid mode");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(!error.to_string().contains("workspace"));
    }

    #[test]
    fn execute_command_validates_test_module_before_trying_workspace_lock() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);

        let error = execute_command(
            &config,
            &Command::Test(TestArgs {
                full: false,
                no_build: false,
                client_mode: None,
                launch: TestLaunchOptionsArgs::default(),
                runner: TestRunner::Yaxunit(TestYaxunitArgs {
                    scope: TestScope::Module {
                        name: "   ".to_owned(),
                    },
                }),
            }),
            None,
            &presenter,
            false,
            false,
        )
        .expect_err("invalid module");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(!error.to_string().contains("workspace"));
    }

    #[test]
    fn execute_command_does_not_clean_logs_when_workspace_is_busy() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let logs_dir = platform_logs_dir(&config.work_path).expect("logs dir");
        fs::create_dir_all(&logs_dir).expect("create logs dir");
        let stale_log = logs_dir.join("stale.log");
        fs::write(&stale_log, "old").expect("stale log");
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let presenter = Presenter::new("text".to_owned(), ColorMode::Disabled);

        let _ = execute_command(
            &config,
            &Command::Build(BuildArgs {
                full_rebuild: true,
                source_set: None,
            }),
            None,
            &presenter,
            true,
            false,
        )
        .expect_err("busy workspace");

        assert!(stale_log.exists());
    }

    #[test]
    fn pre_dispatch_json_error_keeps_command_identity() {
        let error = UseCaseError::new(UseCaseErrorKind::Runtime, "workspace is busy");
        for (command, expected) in [
            (CommandName::Build, "push"),
            (CommandName::Load, "upload"),
            (CommandName::Dump, "pull"),
            (CommandName::Test, "test"),
            (CommandName::Artifacts, "make"),
            (CommandName::Launch, "launch"),
        ] {
            let envelope = pre_dispatch_error_envelope(command.as_str(), &error);
            let json = serde_json::to_value(envelope).expect("json");

            assert_eq!(json["command"], expected);
            assert_eq!(json["data"]["message"], "workspace is busy");
            assert_eq!(json["error"]["code"], "runtime_failure");
        }
    }

    #[test]
    fn infobase_pre_dispatch_phase_distinguishes_lock_from_workspace_preparation() {
        assert_eq!(
            infobase_pre_dispatch_execution_phase(false),
            InfobaseTransferPhase::WorkspaceLock
        );
        assert_eq!(
            infobase_pre_dispatch_execution_phase(true),
            InfobaseTransferPhase::WorkspacePreparation
        );
    }

    #[test]
    fn pre_dispatch_json_error_supports_config_init_identity() {
        let error = UseCaseError::new(UseCaseErrorKind::Validation, "bad config init request");
        let envelope = pre_dispatch_error_envelope("init", &error);
        let json = serde_json::to_value(envelope).expect("json");

        assert_eq!(json["command"], "init");
        assert_eq!(json["data"]["message"], "bad config init request");
        assert_eq!(json["error"]["code"], "invalid_argument");
    }

    #[test]
    fn load_json_message_preserves_success_text_and_all_diagnostics() {
        let result = LoadResult {
            provider: None,
            provider_dispatched: true,
            mode: LoadMode::Load,
            artifact_path: PathBuf::from("main.cf"),
            artifact_type: ArtifactBuildMode::ConfigurationCf,
            extension: None,
            duration_ms: 17,
            execution: ExecutionOutcome::new(ExecutionStatus::Succeeded)
                .with_diagnostics(vec![
                    "deferred cancellation during apply".to_owned(),
                    "deferred timeout during update_db_cfg".to_owned(),
                ])
                .with_payload(LoadExecutionMetadata {
                    applied: true,
                    target_kind: LoadTargetKind::Configuration,
                    compatibility_state: CompatibilityState::NotEstablished,
                    update_db_cfg_ran: true,
                }),
        };

        let json = serde_json::to_value(build_load_envelope(&result)).expect("json");
        let message = json["data"]["message"].as_str().expect("message");

        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["ok"], true);
        assert!(message.contains("load main.cf applied successfully after NotEstablished"));
        assert!(message.contains("deferred cancellation during apply"));
        assert!(message.contains("deferred timeout during update_db_cfg"));
    }
}
