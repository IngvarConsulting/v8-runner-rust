use std::io::IsTerminal;

use clap::{CommandFactory, FromArgMatches};
use serde::Serialize;
use tracing::{debug, error};

use crate::cli::args::{
    BootstrapArgs, Cli, Command, ConfigCommand, ConfigInitArgs, McpCommand, McpServeTransport,
    ToolsCommand,
};
use crate::cli::execute;
use crate::cli::global_flags;
use crate::cli::output::{failure_envelope, print_command_error};
use crate::command_envelope::Envelope;
use crate::config::loader::{
    load_config, load_config_for_infobase_export, load_config_for_prepared_test,
    load_config_for_preview, load_config_for_tools_download, resolve_primary_config_path,
};
use crate::output::presenter::Presenter;
use crate::output::text::{TimelineItem, TimelineStatus};
use crate::support::error::AppError;
use crate::use_cases::config_init::{
    ConfigFormatRequest, ConfigInitRequest, DeclaredOrigin, OriginKey,
};
use crate::use_cases::context::CommandName;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Новые имена команд словаря, у которых внутри остался прежний путь разбора: `init` —
/// это `config init`, `download` — `infobase configuration export`. Обе формы разбираются
/// `clap` и здесь сводятся к одной, чтобы ниже по течению имя было одно.
fn canonical_command(command: Command) -> Command {
    use crate::cli::args::{
        ConfigArgs, ConfigCommand, InfobaseArgs, InfobaseCommand, InfobaseConfigurationArgs,
        InfobaseConfigurationCommand,
    };
    match command {
        Command::ConfigInit(args) => Command::Config(ConfigArgs {
            command: ConfigCommand::Init(args),
        }),
        Command::Download(args) => Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Configuration(InfobaseConfigurationArgs {
                command: InfobaseConfigurationCommand::Export(args),
            }),
        }),
        // Создание базы — свой сценарий, а не выгрузка: путь `infobase create` сводится
        // к внутреннему варианту, и ниже по течению команда остаётся прежней.
        Command::Infobase(InfobaseArgs {
            command: InfobaseCommand::Create,
        }) => Command::Init,
        command => command,
    }
}

/// Отказ по глобальному ключу случается до того, как команда выбрала себе вывод. У
/// `mcp serve` stdout занят протоколом, поэтому его отказы уходят голой строкой в stderr —
/// как и остальные отказы запуска сервера; у прочих команд отказ рисует конверт.
fn render_startup_refusal(
    leaf: &str,
    command: &Command,
    no_color: bool,
    output_format: &str,
    error: UseCaseError,
) -> i32 {
    if leaf.starts_with("mcp ") {
        eprintln!("{error}");
        return error.exit_code();
    }
    let presenter = Presenter::new(output_format.to_owned(), color_mode(no_color));
    let message = error.to_string();
    // Конверт называет команду тем же именем, что и остальные её ответы; лист назван внутри
    // сообщения, где он и нужен читателю.
    print_command_error(&presenter, command_name(command), &error, &message);
    error.exit_code()
}

const BOOTSTRAP_COMMAND: &str = "clone";
const CONFIG_INIT_COMMAND: &str = "init";
const VERSION_COMMAND: &str = "version";

pub fn run() -> i32 {
    let mut matches = Cli::command().get_matches();
    // Путь листа читается до нормализации имён: `clap` уже свёл синонимы к каноническому
    // имени, а `canonical_command` ниже схлопывает разные листья в один вариант.
    let leaf = global_flags::leaf_command_path(&matches);
    let mut cli = match Cli::from_arg_matches_mut(&mut matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    cli.command = canonical_command(cli.command);
    let output_format = cli_output_format(cli.json_message);
    if let Some(error) =
        global_flags::refusal(&leaf, cli.dry_run, cli.infobase.as_deref()).or_else(|| {
            // Очистка рабочего каталога и превью об одном каталоге спорят одинаково у всякой
            // команды, поэтому спрашивается это один раз и до того, как каталог тронут.
            (cli.dry_run && cli.clean_before_execution).then(|| {
                UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    "--clean-before-execution cannot be combined with --dry-run because preview must not modify workPath",
                )
            })
        })
    {
        return render_startup_refusal(&leaf, &cli.command, cli.no_color, output_format, error);
    }

    if let Command::Version = &cli.command {
        return run_version_command(output_format);
    }

    if let Command::Mcp(args) = &cli.command {
        return run_mcp_command(&cli, args);
    }

    let color_mode = color_mode(cli.no_color);
    let mut presenter = Presenter::new(output_format.to_owned(), color_mode);

    if let Command::Config(args) = &cli.command {
        return run_config_command(args, &cli, &presenter);
    }

    if let Command::Bootstrap(args) = &cli.command {
        return run_bootstrap(args, &cli, &presenter);
    }

    if let Command::Infobase(args) = &cli.command {
        if let Err(error) = execute::validate_infobase_request(args) {
            let error =
                execute::render_invalid_infobase_request(args, &presenter, error, cli.dry_run);
            return error.exit_code();
        }
    }

    if let Command::Extensions(args) = &cli.command {
        if let Err(message) = args.validate_property_options() {
            let error = UseCaseError::new(UseCaseErrorKind::Validation, message);
            print_command_error(&presenter, command_name(&cli.command), &error, message);
            return error.exit_code();
        }
    }

    let config = match load_cli_config(&cli) {
        Ok(loaded) => {
            presenter.note_load_warnings(
                cli.config
                    .as_deref()
                    .unwrap_or(crate::config::loader::DEFAULT_CONFIG_FILE_NAME),
                &loaded.warnings,
            );
            loaded.config
        }
        Err(e) => {
            let message = e.to_string();
            let error = UseCaseError::from(AppError::from(e));
            if let Command::Infobase(args) = &cli.command {
                let error = execute::render_infobase_pre_dispatch_failure(
                    args,
                    &presenter,
                    error,
                    "provider selection was not attempted because configuration loading failed",
                    crate::domain::infobase_export::InfobaseTransferPhase::ConfigurationLoad,
                    cli.dry_run,
                );
                return error.exit_code();
            }
            print_command_error(&presenter, command_name(&cli.command), &error, &message);
            return error.exit_code();
        }
    };
    let mut prepared_infobase = match &cli.command {
        Command::Infobase(args) => {
            match execute::prepare_infobase_cli_command(&config, args, &presenter, cli.dry_run) {
                Ok(prepared) => Some(prepared),
                Err(error) => return error.exit_code(),
            }
        }
        _ => None,
    };
    if let Command::Infobase(_) = &cli.command {
        if cli.dry_run {
            return match execute::preview_prepared_infobase_command(
                &config,
                prepared_infobase
                    .take()
                    .expect("infobase command was prepared before preview"),
                &presenter,
            ) {
                Ok(()) => 0,
                Err(error) => error.exit_code(),
            };
        }
    }
    let primary_config_path = match resolve_primary_config_path(cli.config.as_deref()) {
        Ok(path) => path,
        Err(e) => {
            let message = e.to_string();
            let error = UseCaseError::from(AppError::from(e));
            print_command_error(&presenter, command_name(&cli.command), &error, &message);
            return error.exit_code();
        }
    };

    let level = cli.log_level.as_deref().unwrap_or("info");
    let is_infobase_command = matches!(&cli.command, Command::Infobase(_));
    let logging_result = if is_infobase_command {
        crate::support::logging::init_action_logging_deferred(
            level,
            output_format,
            color_enabled(cli.no_color),
            &config.work_path,
            cli.dry_run,
        )
    } else {
        crate::support::logging::init_action_logging(
            level,
            output_format,
            color_enabled(cli.no_color),
            &config.work_path,
            cli.dry_run,
        )
    };
    let action_log_path = match logging_result {
        Ok(path) => path,
        Err(e) => {
            let message = e.to_string();
            let error = UseCaseError::new(UseCaseErrorKind::Runtime, message.clone());
            print_command_error(&presenter, command_name(&cli.command), &error, &message);
            return error.exit_code();
        }
    };

    if !is_infobase_command {
        debug!(
            command = command_name(&cli.command),
            output = output_format,
            work_path = %config.work_path.display(),
            "starting command"
        );
        if let Some(path) = &action_log_path {
            debug!(path = %path.display(), "action log file enabled");
        }
    }

    let result = match &cli.command {
        Command::Version => unreachable!("version command is handled before config loading"),
        Command::Bootstrap(_) => unreachable!("bootstrap command is handled before config loading"),
        Command::ConfigInit(_) | Command::Download(_) => {
            unreachable!("new command names are normalised in canonical_command")
        }
        Command::Init
        | Command::Config(_)
        | Command::Tools(_)
        | Command::Extensions(_)
        | Command::Build(_)
        | Command::Load(_)
        | Command::Test(_)
        | Command::Dump(_)
        | Command::Convert(_)
        | Command::Artifacts(_)
        | Command::Syntax(_)
        | Command::Launch(_)
        | Command::Publish(_) => execute::execute_command(
            &config,
            &cli.command,
            Some(primary_config_path),
            &presenter,
            cli.clean_before_execution,
            cli.dry_run,
        ),
        Command::Infobase(_) => execute::execute_prepared_infobase_command(
            &config,
            prepared_infobase
                .take()
                .expect("infobase command was prepared before action logging"),
            &presenter,
            cli.clean_before_execution,
        ),
        Command::Mcp(_) => unreachable!("mcp commands are handled before CLI presenter setup"),
    };

    match result {
        Ok(()) => {
            if !is_infobase_command {
                debug!(
                    command = command_name(&cli.command),
                    "command finished successfully"
                );
            }
            0
        }
        Err(e) => {
            // Text command adapters have already rendered the error; text action logs
            // go to stdout, so logging here would duplicate user-facing output.
            if presenter.is_json() && !is_infobase_command {
                error!("{e}");
            }
            e.exit_code()
        }
    }
}

fn load_cli_config(
    cli: &Cli,
) -> Result<crate::config::loader::LoadedConfig, crate::config::loader::ConfigLoadError> {
    let selector = crate::config::model::InfobaseSelector::from_flag(cli.infobase.as_deref());
    let config_path = cli.config.as_deref();
    let workdir = cli.workdir.as_deref();
    if matches!(
        &cli.command,
        Command::Tools(crate::cli::args::ToolsArgs {
            command: ToolsCommand::Download(_)
        })
    ) {
        load_config_for_tools_download(config_path, workdir, &selector)
    } else if execute::uses_infobase_export_config(&cli.command) {
        load_config_for_infobase_export(config_path, workdir, &selector)
    } else if matches!(&cli.command, Command::Test(args) if args.no_build) {
        load_config_for_prepared_test(config_path, workdir, &selector)
    } else if cli.dry_run {
        // Превью не создаёт рабочего каталога: проверки те же, готовит `workPath` только
        // применение. Прежде так загружалось одно лишь `extensions --dry-run`.
        load_config_for_preview(config_path, workdir, &selector)
    } else {
        load_config(config_path, workdir, &selector)
    }
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Version => VERSION_COMMAND,
        Command::Bootstrap(_) => BOOTSTRAP_COMMAND,
        Command::Config(_) | Command::ConfigInit(_) => CONFIG_INIT_COMMAND,
        // Сервер отвечает голой строкой в stderr, имени конверта ему не нужно; здесь оно
        // есть, чтобы имя было у всякой команды.
        Command::Mcp(_) => "mcp serve",
        _ => execute::command_name(command).as_str(),
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct VersionInfo {
    pub name: &'static str,
    pub version: &'static str,
}

fn run_version_command(output_format: &str) -> i32 {
    let info = VersionInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    };

    let presenter = Presenter::new(
        output_format.to_owned(),
        crate::output::presenter::ColorMode::Disabled,
    );
    if presenter.is_json() {
        presenter.print_envelope(&Envelope::ok(VERSION_COMMAND, 0, info));
    } else {
        presenter.print_bare(&format!("{} {}", info.name, info.version));
    }

    0
}

fn run_config_command(
    args: &crate::cli::args::ConfigArgs,
    cli: &Cli,
    presenter: &Presenter,
) -> i32 {
    match &args.command {
        ConfigCommand::Init(init_args) => run_config_init(init_args, cli, presenter),
    }
}

fn run_bootstrap(args: &BootstrapArgs, cli: &Cli, presenter: &Presenter) -> i32 {
    if config_flag_was_explicitly_set() {
        let message =
            "global --config flag is not supported for `clone`; use `clone --project-dir <DIR>` to choose where the generated project is written";
        let error = UseCaseError::new(UseCaseErrorKind::Validation, message);
        print_command_error(presenter, BOOTSTRAP_COMMAND, &error, message);
        return error.exit_code();
    }

    let project_dir = match resolve_bootstrap_project_dir(args.project_dir.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            let message = error.to_string();
            let error = UseCaseError::from(error);
            print_command_error(presenter, BOOTSTRAP_COMMAND, &error, &message);
            return error.exit_code();
        }
    };
    let work_path = project_dir.join("build");
    let level = cli.log_level.as_deref().unwrap_or("info");
    if let Err(error) = crate::support::logging::init_action_logging(
        level,
        if presenter.is_json() { "json" } else { "text" },
        color_enabled(cli.no_color),
        &work_path,
        cli.dry_run,
    ) {
        let message = error.to_string();
        let error = UseCaseError::new(UseCaseErrorKind::Runtime, message.clone());
        print_command_error(presenter, BOOTSTRAP_COMMAND, &error, &message);
        return error.exit_code();
    }

    let request = crate::use_cases::bootstrap_project::BootstrapRequest {
        project_dir,
        connection: args.connection.clone(),
        platform_version: args.platform_version.clone(),
        platform_path: args.platform_path.clone().map(Into::into),
        user: args.user.clone(),
        password: args.password.clone(),
        source_dir: args.source_dir.clone().into(),
        force: args.force,
        dry_run: cli.dry_run,
    };
    // Ctrl+C и SIGTERM — отмена, как у остальных команд: по умолчанию сигнал убил бы
    // раннер под замком, и файл владельца замка пережил бы его.
    let cancellation = tokio_util::sync::CancellationToken::new();
    let _signal_guard = crate::cli::signal::CliSignalGuard::install(cancellation.clone());
    let context = crate::use_cases::context::ExecutionContext::cli(CommandName::Bootstrap)
        .with_cancellation(cancellation);
    let outcome = crate::use_cases::bootstrap_project::plan(request).and_then(|plan| {
        // Замок берётся по настройкам плана до первого файла проекта: занятый каталог
        // отказывает, пока проекта ещё нет. Внешняя ошибка — только отказ замка, итог
        // клона внутри.
        // `--clean-before-execution` клон не чистит: журналов платформы у нового проекта нет.
        let clean_before_execution = false;
        let preview = plan.is_preview();
        execute::with_cli_workspace_lock(
            plan.config(),
            presenter,
            CommandName::Bootstrap,
            clean_before_execution,
            preview,
            || {
                Ok(crate::use_cases::bootstrap_project::execute(
                    &context, &plan,
                ))
            },
        )
        .unwrap_or_else(|error| {
            Err(crate::use_cases::result::UseCaseFailure::without_payload(
                error,
            ))
        })
    });
    match outcome {
        Ok(result) => {
            if presenter.is_json() {
                presenter.print_envelope(&Envelope {
                    ok: true,
                    command: BOOTSTRAP_COMMAND.to_owned(),
                    duration_ms: result.duration_ms,
                    warnings: result.warnings.clone(),
                    steps: Vec::new(),
                    error: None,
                    data: result,
                });
            } else {
                render_bootstrap_text(
                    &result,
                    presenter,
                    true,
                    execute::Requested::from_dry_run(cli.dry_run),
                );
            }
            0
        }
        Err(failure) => {
            let error = failure.error;
            if presenter.is_json() {
                if let Some(result) = failure.payload {
                    presenter.print_envelope(&failure_envelope(
                        BOOTSTRAP_COMMAND,
                        result.duration_ms,
                        result,
                        &error,
                    ));
                } else {
                    presenter.print_envelope(&failure_envelope(
                        BOOTSTRAP_COMMAND,
                        0,
                        crate::cli::output::RefusalData {
                            message: error.message().to_owned(),
                        },
                        &error,
                    ));
                }
            } else {
                if let Some(result) = failure.payload.as_ref() {
                    render_bootstrap_text(
                        result,
                        presenter,
                        false,
                        execute::Requested::from_dry_run(cli.dry_run),
                    );
                }
                presenter.print_error(&error.to_string());
            }
            error.exit_code()
        }
    }
}

fn resolve_bootstrap_project_dir(
    project_dir: Option<&str>,
) -> Result<std::path::PathBuf, AppError> {
    match project_dir {
        Some(path) => Ok(path.into()),
        None => std::env::current_dir().map_err(|error| {
            AppError::Runtime(format!("failed to resolve current directory: {error}"))
        }),
    }
}

fn run_config_init(args: &ConfigInitArgs, cli: &Cli, presenter: &Presenter) -> i32 {
    if config_flag_was_explicitly_set() {
        let message =
            "global --config flag is not supported for `init`; use `init --output <FILE>` to choose where the generated config is written";
        let error = UseCaseError::new(UseCaseErrorKind::Validation, message);
        print_command_error(presenter, CONFIG_INIT_COMMAND, &error, message);
        return error.exit_code();
    }

    let project_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            let message = format!("failed to resolve current directory: {error}");
            let error = UseCaseError::new(UseCaseErrorKind::Runtime, message.clone());
            print_command_error(presenter, CONFIG_INIT_COMMAND, &error, &message);
            return error.exit_code();
        }
    };
    let output_path = args.output.as_deref().unwrap_or("v8project.yaml");

    // `init` объявляет базу, а не выбирает её: адрес приходит либо своим ключом команды,
    // либо глобальным `--infobase` — сайт называет `init --infobase <строка>` рецептом
    // объявления `origin`. Два ключа об одном адресе — отказ, а не тихий выбор одного.
    let declared_by_flag =
        match crate::config::model::InfobaseSelector::from_flag(cli.infobase.as_deref()) {
            crate::config::model::InfobaseSelector::Connection(connection) => Some(connection),
            _ => None,
        };
    let own_key = args
        .connection
        .clone()
        .filter(|connection| !connection.trim().is_empty());
    let connection = match (own_key, declared_by_flag) {
        (Some(_), Some(_)) => {
            let message =
                "--connection and --infobase name the same address for `init`; pass one of them";
            let error = UseCaseError::new(UseCaseErrorKind::Validation, message);
            print_command_error(presenter, CONFIG_INIT_COMMAND, &error, message);
            return error.exit_code();
        }
        (Some(connection), None) => Some(DeclaredOrigin {
            key: OriginKey::Connection,
            connection,
        }),
        (None, Some(connection)) => Some(DeclaredOrigin {
            key: OriginKey::Infobase,
            connection,
        }),
        (None, None) => None,
    };

    let request = ConfigInitRequest {
        project_dir,
        output_path: output_path.into(),
        force: args.force,
        connection,
        format: map_config_format(&args.format),
    };

    match crate::use_cases::config_init::execute(&request) {
        Ok(result) => {
            if presenter.is_json() {
                presenter.print_envelope(&Envelope {
                    ok: true,
                    command: CONFIG_INIT_COMMAND.to_owned(),
                    duration_ms: result.duration_ms,
                    warnings: result.warnings.clone(),
                    steps: Vec::new(),
                    error: None,
                    data: result,
                });
            } else {
                render_config_init_text(&result, presenter);
            }
            0
        }
        Err(error) => {
            let message = error.to_string();
            let error = UseCaseError::from(error);
            print_command_error(presenter, CONFIG_INIT_COMMAND, &error, &message);
            error.exit_code()
        }
    }
}

fn render_config_init_text(
    result: &crate::domain::config_init::ConfigInitResult,
    presenter: &Presenter,
) {
    let mut details = vec![
        format!("path: {}", result.path),
        format!("local path: {}", result.local_path),
        format!("gitignore: {}", result.gitignore_path),
        format!("format: {}", result.format),
    ];
    if result.overwritten {
        details.push("overwritten: yes".to_owned());
    }
    if let Some(platform_version) = result.platform_version.as_deref() {
        details.push(format!("platform version: {platform_version}"));
    }
    for source_set in &result.source_sets {
        details.push(format!(
            "source-set {}: {} ({})",
            source_set.name, source_set.path, source_set.source_type
        ));
    }
    for warning in &result.warnings {
        details.push(format!("[warning] {warning}"));
    }

    let completion = if result.warnings.is_empty() {
        "Config written successfully"
    } else {
        "Config written with warnings"
    };
    let timeline = vec![
        TimelineItem::new(TimelineStatus::Succeeded, "config:").with_detail(details.join("\n")),
        TimelineItem::new(TimelineStatus::Succeeded, completion),
    ];
    presenter.print_timeline(&timeline);
}

fn render_bootstrap_text(
    result: &crate::domain::bootstrap::BootstrapResult,
    presenter: &Presenter,
    succeeded: bool,
    requested: execute::Requested,
) {
    let mut details = vec![
        format!("path: {}", result.path.display()),
        format!("local path: {}", result.local_path.display()),
        format!("gitignore: {}", result.gitignore_path.display()),
        format!("source dir: {}", result.source_dir.display()),
        format!("dumped: {}", if result.dumped { "yes" } else { "no" }),
        format!(
            "provider dispatched: {}",
            if result.provider_dispatched {
                "yes"
            } else {
                "no"
            }
        ),
    ];
    if let Some(message) = result.message.as_deref() {
        details.push(format!(
            "{}: {message}",
            if succeeded { "message" } else { "error" }
        ));
    }
    for warning in &result.warnings {
        details.push(format!("[warning] {warning}"));
    }

    // Превью проекта не заводит, и называть его заведённым нельзя: пути в ответе — те,
    // что были бы написаны. Признак берётся у запроса, а не выводится из
    // `provider_dispatched`: тот говорит, получил ли исполнитель работу, а не было ли
    // превью, и успешный боевой прогон может вернуться без работы исполнителю.
    let label = match (succeeded, requested) {
        (true, execute::Requested::Apply) => "Project cloned successfully",
        (true, execute::Requested::Preview) => "Project clone planned, nothing written",
        (false, _) => "Project clone failed",
    };
    let timeline = vec![
        TimelineItem::new(
            if succeeded {
                TimelineStatus::Succeeded
            } else {
                TimelineStatus::Failed
            },
            "clone:",
        )
        .with_detail(details.join("\n")),
        TimelineItem::new(
            if succeeded {
                TimelineStatus::Succeeded
            } else {
                TimelineStatus::Failed
            },
            label,
        ),
    ];
    presenter.print_timeline(&timeline);
}

fn map_config_format(value: &str) -> ConfigFormatRequest {
    match value {
        "designer" | "DESIGNER" => ConfigFormatRequest::Designer,
        "edt" | "EDT" => ConfigFormatRequest::Edt,
        _ => ConfigFormatRequest::Auto,
    }
}

fn run_mcp_command(cli: &Cli, args: &crate::cli::args::McpArgs) -> i32 {
    match &args.command {
        McpCommand::Serve(serve) => match serve.transport {
            McpServeTransport::Stdio => run_mcp_stdio(cli),
            McpServeTransport::Http => run_mcp_http(cli),
        },
    }
}

fn run_mcp_stdio(cli: &Cli) -> i32 {
    install_mcp_panic_hook();

    let config = match prepare_mcp_runtime(cli, "stdio") {
        Ok(config) => config,
        Err(exit_code) => return exit_code,
    };

    match crate::mcp::server::serve_stdio(config) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            crate::output::exit_codes::RUNTIME_ERROR
        }
    }
}

fn run_mcp_http(cli: &Cli) -> i32 {
    install_mcp_panic_hook();

    let config = match prepare_mcp_runtime(cli, "http") {
        Ok(config) => config,
        Err(exit_code) => return exit_code,
    };

    match crate::mcp::server::serve_http(config) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            crate::output::exit_codes::RUNTIME_ERROR
        }
    }
}

fn prepare_mcp_runtime(
    cli: &Cli,
    transport: &'static str,
) -> Result<crate::config::model::AppConfig, i32> {
    let selector = crate::config::model::InfobaseSelector::from_flag(cli.infobase.as_deref());
    let loaded = match load_config(cli.config.as_deref(), cli.workdir.as_deref(), &selector) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("{error}");
            return Err(crate::output::exit_codes::VALIDATION_ERROR);
        }
    };
    let config = loaded.config;

    if cli.clean_before_execution {
        eprintln!("--clean-before-execution is not supported for MCP transports");
        return Err(crate::output::exit_codes::VALIDATION_ERROR);
    }

    let level = cli.log_level.as_deref().unwrap_or("info");
    if let Err(error) =
        // Сервер превью не предлагает: ключа нет в опубликованной поверхности.
        crate::support::logging::init_action_logging(
            level,
            "json",
            false,
            &config.work_path,
            false,
        )
    {
        eprintln!("{error}");
        return Err(crate::output::exit_codes::RUNTIME_ERROR);
    }

    // stdout сервера занят протоколом: предупреждения загрузки уходят в журнал действий.
    for warning in &loaded.warnings {
        tracing::warn!(transport, "{warning}");
    }

    debug!(
        transport,
        work_path = %config.work_path.display(),
        "starting mcp server"
    );

    Ok(config)
}

fn install_mcp_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("{panic_info}");
    }));
}

/// Цвет — оформление, а не содержание, и включается он только там, где его увидят.
///
/// Перенаправленный вывод читает не терминал: escape-последовательности в файле журнала
/// или в выводе CI мешают и человеку, и `grep`. Поэтому цвет выключает и флаг, и
/// договорённость `NO_COLOR`, и сам факт, что на том конце не терминал.
/// Цвет включается только там, где его увидят.
///
/// Порядок как у остальных инструментов: запрет сильнее разрешения, а разрешение
/// сильнее догадки. `FORCE_COLOR` нужен тем, кто перенаправляет вывод в средство,
/// которое ANSI отрисует, — в первую очередь CI.
fn color_enabled(no_color_flag: bool) -> bool {
    if no_color_flag || std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::env::var_os("FORCE_COLOR").is_some() || std::io::stdout().is_terminal()
}

fn color_mode(no_color_flag: bool) -> crate::output::presenter::ColorMode {
    if color_enabled(no_color_flag) {
        crate::output::presenter::ColorMode::Enabled
    } else {
        crate::output::presenter::ColorMode::Disabled
    }
}

fn cli_output_format(json_message: bool) -> &'static str {
    if json_message {
        "json"
    } else {
        "text"
    }
}

fn config_flag_was_explicitly_set() -> bool {
    std::env::args_os().skip(1).any(|arg| {
        let value = arg.to_string_lossy();
        value == "--config" || value.starts_with("--config=")
    })
}
