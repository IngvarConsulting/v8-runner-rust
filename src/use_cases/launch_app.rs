use std::path::Path;
use std::time::Duration;

use crate::config::model::AppConfig;
use crate::domain::launch::{
    ExternalEpfWaitResult, LaunchMode, LaunchPlan, LaunchResult, LaunchVia, PlatformResolution,
    PlatformResolutionSource,
};
use crate::domain::runner::{launch_key_alias_matches, LaunchOptions};
use crate::platform::enterprise::{
    build_launch_args, normalize_launch_payload_path, LaunchAddress, LaunchClientMode,
};
use crate::platform::locator::{ResolutionSource, UtilityLocation, UtilityType, UtilityVersion};
use crate::platform::process::{ManagedSpawnMode, ProcessRequest};
use crate::platform::secrets::{mask_preview_args, mask_url_userinfo};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::client_mcp_readiness;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::launch_keys::vanessa_enterprise_launch_keys;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::{
    ClientMcpAddonRequest, ClientMcpMode, ClientMcpOptionsRequest, EnterpriseLaunchTarget,
    LaunchRequest as LaunchArgs, LaunchTargetRequest,
};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use crate::use_cases::tool_extension;
use tracing::debug;

const LAUNCH_STARTUP_PROBE: Duration = Duration::from_millis(250);

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LaunchArgs,
) -> UseCaseResult<LaunchResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        target = ?args.target,
        "executing launch use case"
    );
    if args.target == LaunchTargetRequest::Web {
        return execute_web(context, config, args);
    }
    let (mode, utility, client_mode) = match args.target {
        LaunchTargetRequest::Web => unreachable!("web launches are handled above"),
        LaunchTargetRequest::Designer => (
            LaunchMode::Designer,
            UtilityType::V8,
            LaunchClientMode::Designer,
        ),
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThinClient) => {
            (LaunchMode::Thin, UtilityType::V8C, LaunchClientMode::Thin)
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThickClient) => {
            (LaunchMode::Thick, UtilityType::V8, LaunchClientMode::Thick)
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::OrdinaryApplication) => (
            LaunchMode::Ordinary,
            UtilityType::V8,
            LaunchClientMode::Ordinary,
        ),
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { mode }) => {
            client_mcp_launch_shape(mode)
        }
    };

    // Прямой шлюз автономной цели раннер пока не использует (#205), поэтому её
    // открывает только клиентский адрес — а по нему ходит только тонкий клиент.
    // Конфигуратор, толстый и обычный отказывают здесь ровно так же, как отказывали до
    // появления второго пути.
    let standalone = config.target_kind() == crate::domain::capability::TargetKind::Standalone;
    if standalone && !matches!(client_mode, LaunchClientMode::Thin) {
        return Err(UseCaseFailure::without_payload(
            AppError::CapabilityUnavailable(
                "a standalone server is opened by its web address: use `launch web` with infobase.web.url; a client is not launched against the gate".to_owned(),
            ),
        ));
    }

    if let Some(interruption) = context.interruption() {
        return Err(UseCaseFailure::without_payload(AppError::Runtime(
            crate::use_cases::interruption::command_interruption_message(context, interruption),
        )));
    }

    // Путь и адрес разрешаются до поиска утилиты: искать платформу, когда адреса нет,
    // незачем, а отказ про адрес человеку понятнее отказа про платформу.
    let via = resolve_launch_via(args.via, client_mode, standalone)
        .map_err(UseCaseFailure::without_payload)?;
    let web_url = match via {
        LaunchVia::Connection => None,
        LaunchVia::Web => Some(
            client_address(config)
                .map_err(UseCaseFailure::without_payload)?
                .to_owned(),
        ),
    };

    // В ответ и в план адрес идёт без пароля из userinfo: argv несёт настоящий,
    // отчёт — замаскированный.
    let reported_url = web_url.as_deref().map(mask_url_userinfo);

    let launch = effective_launch_options(config, args).map_err(UseCaseFailure::without_payload)?;
    let external_epf_wait =
        external_epf_wait_plan(config, args, &launch).map_err(UseCaseFailure::without_payload)?;
    let readiness_url =
        client_mcp_readiness_url(config, args).map_err(UseCaseFailure::without_payload)?;
    if args.dry_run {
        // Both options report an outcome observed from a running client, which a preview
        // never starts; answering them with a plan would be a fabricated observation.
        let conflicting = if external_epf_wait.is_some() {
            Some("--wait-for-exit")
        } else if readiness_url.is_some() {
            Some("--wait-ready")
        } else {
            None
        };
        if let Some(option) = conflicting {
            return Err(UseCaseFailure::without_payload(AppError::Validation(
                format!(
                    "{option} cannot be combined with launch --dry-run because a preview never starts the client"
                ),
            )));
        }
    }

    let additional_launch_keys = effective_enterprise_launch_keys(config, args, &launch);
    let mut utilities = PlatformUtilities::from_config(config);
    let location = utilities
        .locate(utility)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let platform_resolution = Some(platform_resolution(&location));
    let connection = config.v8_connection();
    let address = match &web_url {
        None => LaunchAddress::Connection(&connection),
        // У автономной цели `infobase.user`/`password` — учётные данные SSH-шлюза, а не
        // базы: клиенту они не принадлежат и в его командную строку не попадают.
        Some(url) => LaunchAddress::Web {
            url,
            credentials: (!standalone).then_some(&connection),
        },
    };
    let process_request = ProcessRequest {
        program: location.path.clone(),
        args: build_launch_args(client_mode, address, &additional_launch_keys, &launch),
        workdir: None,
        stdout_log_path: None,
        stderr_log_path: external_epf_wait
            .as_ref()
            .map(|plan| plan.stderr_path.clone()),
        startup_probe: external_epf_wait
            .as_ref()
            .map(|_| None)
            .unwrap_or(Some(LAUNCH_STARTUP_PROBE)),
    };

    if args.dry_run {
        let connection = config.v8_connection();
        let secrets: Vec<&str> = connection.password.as_deref().into_iter().collect();
        let masked = mask_preview_args(&process_request.args, &secrets);
        log_live_stage(
            "launch: preview",
            "[Launch] preview only, client process not dispatched",
        );
        return Ok(LaunchResult {
            ok: true,
            mode,
            pid: None,
            via,
            binary: location.path.clone(),
            platform_resolution,
            url: reported_url.clone(),
            provider_dispatched: false,
            plan: Some(LaunchPlan {
                program: process_request.program.clone(),
                args: masked,
            }),
            message: Some(preview_message(config, args, &location.path)),
            mcp_readiness: None,
            external_epf_wait: None,
        });
    }

    debug!("[Запуск] Приложение: {}", mode_label(args.target));
    log_live_stage("launch: start", "[Launch] starting client process");
    let runner = utilities.runner_for(utility);

    if let Some(plan) = external_epf_wait {
        let managed = runner
            .spawn_managed(&process_request, ManagedSpawnMode::Wait)
            .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
        let pid = managed.pid();
        let outcome = managed
            .wait_for_exit(&context.process_policy(
                InterruptionSafetyClass::GracefulThenKill,
                Some(Duration::from_millis(plan.timeout_ms)),
            ))
            .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
        let message = if outcome.timed_out {
            format!(
                "External EPF client timed out after {}ms and was terminated",
                plan.timeout_ms
            )
        } else {
            format!(
                "External EPF client exited with status {}",
                outcome.exit_code.unwrap_or(-1)
            )
        };
        let result = LaunchResult {
            ok: !outcome.timed_out,
            mode,
            via,
            pid: Some(pid),
            binary: location.path,
            platform_resolution,
            url: reported_url.clone(),
            provider_dispatched: true,
            plan: None,
            message: Some(message.clone()),
            mcp_readiness: None,
            external_epf_wait: Some(ExternalEpfWaitResult {
                pid,
                execute_path: plan.execute_path,
                exit_code: outcome.exit_code,
                timed_out: outcome.timed_out,
                output_path: plan.output_path,
                stderr_path: plan.stderr_path.display().to_string(),
            }),
        };
        if outcome.timed_out {
            return Err(UseCaseFailure::with_payload(
                AppError::Runtime(message),
                result,
            ));
        }
        return Ok(result);
    }

    if let Some(url) = readiness_url {
        let managed = runner
            .spawn_managed(&process_request, ManagedSpawnMode::Detached)
            .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
        let pid = managed.pid();
        let binary = managed.binary().clone();
        let mut result = LaunchResult {
            ok: true,
            mode,
            via,
            pid: Some(pid),
            binary: binary.clone(),
            platform_resolution: platform_resolution.clone(),
            url: reported_url.clone(),
            provider_dispatched: true,
            plan: None,
            message: Some(launch_message(config, args, &binary, pid)),
            mcp_readiness: None,
            external_epf_wait: None,
        };
        let required_tools = required_mcp_tools(args);
        match client_mcp_readiness::wait_for_readiness(
            context,
            &url,
            required_tools,
            config.client_mcp_wait_ready_timeout_duration(),
        ) {
            Ok(readiness) => {
                result.mcp_readiness = Some(readiness);
                let _ = managed.detach();
                return Ok(result);
            }
            Err(readiness) => {
                let message = readiness
                    .message
                    .clone()
                    .unwrap_or_else(|| "MCP endpoint did not become ready".to_owned());
                managed.terminate();
                result.ok = false;
                result.message = Some(format!(
                    "Launched {} via {} (pid {}) but {message}; process terminated",
                    mode_label(args.target),
                    binary.display(),
                    pid
                ));
                result.mcp_readiness = Some(readiness);
                return Err(UseCaseFailure::with_payload(
                    AppError::Runtime(message),
                    result,
                ));
            }
        }
    }

    let spawned = runner
        .spawn(&process_request)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;

    Ok(LaunchResult {
        ok: true,
        mode,
        pid: Some(spawned.pid),
        via,
        binary: spawned.binary.clone(),
        platform_resolution,
        url: reported_url.clone(),
        provider_dispatched: true,
        plan: None,
        message: Some(launch_message(config, args, &spawned.binary, spawned.pid)),
        mcp_readiness: None,
        external_epf_wait: None,
    })
}

fn platform_resolution(location: &UtilityLocation) -> PlatformResolution {
    PlatformResolution {
        path: location.path.clone(),
        version: location.version.as_ref().map(utility_version_string),
        source: match location.source {
            ResolutionSource::Explicit => PlatformResolutionSource::Explicit,
            ResolutionSource::DefaultRoot => PlatformResolutionSource::DefaultRoot,
            ResolutionSource::Path => PlatformResolutionSource::Path,
        },
        installation_root: location.installation_root.clone(),
    }
}

fn utility_version_string(version: &UtilityVersion) -> String {
    match version {
        UtilityVersion::Platform(version) => version.to_string(),
        UtilityVersion::Edt(version) => version
            .parts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("."),
    }
}

struct ExternalEpfWaitPlan {
    timeout_ms: u64,
    execute_path: String,
    output_path: String,
    stderr_path: std::path::PathBuf,
}

fn external_epf_wait_plan(
    config: &AppConfig,
    args: &LaunchArgs,
    launch: &LaunchOptions,
) -> Result<Option<ExternalEpfWaitPlan>, AppError> {
    let Some(wait) = &launch.external_epf_wait else {
        return Ok(None);
    };
    if !matches!(
        args.target,
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThinClient)
    ) {
        return Err(AppError::Validation(
            "--wait-for-exit is supported only for `launch thin`".to_owned(),
        ));
    }
    let execute = launch.execute.as_deref().ok_or_else(|| {
        AppError::Validation(
            "--wait-for-exit requires an explicit --execute <external.epf>".to_owned(),
        )
    })?;
    if !execute.to_ascii_lowercase().ends_with(".epf") {
        return Err(AppError::Validation(
            "--wait-for-exit requires --execute to name an external .epf file".to_owned(),
        ));
    }
    let output_path = launch
        .out
        .clone()
        .ok_or_else(|| AppError::Validation("--wait-for-exit requires --output".to_owned()))?;
    if launch
        .raw_args
        .iter()
        .chain(config.tools.enterprise.additional_launch_keys.iter())
        .any(|raw| is_wait_reserved_raw_key(raw))
    {
        return Err(AppError::Validation(
            "--wait-for-exit does not support raw /C, /Execute, or /Out launch keys".to_owned(),
        ));
    }
    Ok(Some(ExternalEpfWaitPlan {
        timeout_ms: wait.timeout_ms,
        execute_path: normalize_launch_payload_path(Path::new(execute)),
        output_path,
        stderr_path: std::path::PathBuf::from(&wait.stderr_output),
    }))
}

fn is_wait_reserved_raw_key(raw: &str) -> bool {
    ["c", "execute", "out"]
        .iter()
        .any(|key| launch_key_alias_matches(raw, key))
}

fn client_mcp_readiness_url(
    config: &AppConfig,
    args: &LaunchArgs,
) -> Result<Option<String>, AppError> {
    let Some(client_mcp) = args.client_mcp.as_ref() else {
        return Ok(None);
    };
    if !client_mcp.wait_ready {
        return Ok(None);
    }
    let Some(port) = client_mcp.port.or(config.tools.client_mcp.port) else {
        return Err(AppError::Validation(
            "launch mcp --wait-ready requires --mcp-port or tools.client_mcp.port".to_owned(),
        ));
    };
    Ok(Some(client_mcp_readiness::endpoint_url(port)))
}

fn required_mcp_tools(args: &LaunchArgs) -> &'static [&'static str] {
    if is_client_mcp_va_launch(args) {
        client_mcp_readiness::VANESSA_MCP_TOOLS
    } else {
        &[]
    }
}

fn append_client_mcp_build_hint(message: &mut String, config: &AppConfig, args: &LaunchArgs) {
    if !is_client_mcp_launch(args) {
        return;
    }
    if let Some(hint) = tool_extension::client_mcp_build_hint(config) {
        message.push_str("; ");
        message.push_str(hint);
    }
}

fn preview_message(config: &AppConfig, args: &LaunchArgs, binary: &Path) -> String {
    let mut message = format!(
        "Previewed {} via {}; client process not dispatched",
        mode_label(args.target),
        binary.display()
    );
    append_client_mcp_build_hint(&mut message, config, args);
    message
}

fn launch_message(config: &AppConfig, args: &LaunchArgs, binary: &Path, pid: u32) -> String {
    let mut message = format!(
        "Launched {} via {} (pid {})",
        mode_label(args.target),
        binary.display(),
        pid
    );
    append_client_mcp_build_hint(&mut message, config, args);
    message
}

fn is_client_mcp_launch(args: &LaunchArgs) -> bool {
    matches!(
        args.target,
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. })
    )
}

fn effective_enterprise_launch_keys(
    config: &AppConfig,
    args: &LaunchArgs,
    launch: &LaunchOptions,
) -> Vec<String> {
    if is_client_mcp_va_launch(args) {
        return vanessa_enterprise_launch_keys(
            &config.tools.enterprise.additional_launch_keys,
            launch,
        );
    }
    config.tools.enterprise.additional_launch_keys.clone()
}

fn is_client_mcp_va_launch(args: &LaunchArgs) -> bool {
    args.client_mcp.as_ref().is_some_and(|client_mcp| {
        matches!(
            client_mcp.addon,
            Some(ClientMcpAddonRequest::VanessaAutomation)
        )
    })
}

fn mode_label(target: LaunchTargetRequest) -> &'static str {
    match target {
        LaunchTargetRequest::Designer => "конфигуратор",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThinClient) => "тонкий клиент",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThickClient) => "толстый клиент",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::OrdinaryApplication) => {
            "обычное приложение"
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. }) => {
            "клиентский MCP-сервер"
        }
        LaunchTargetRequest::Web => "веб-клиент",
    }
}

/// `launch web`: открыть объявленный клиентский адрес в браузере.
///
/// Без `infobase.web.url` — типизированный отказ: адрес появляется после публикации
/// или задаётся вручную, выводить его раннер не берётся. Доступность адреса не
/// проверяется: раннер не пингует публикацию и не чинит её.
fn execute_web(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LaunchArgs,
) -> UseCaseResult<LaunchResult> {
    let url = client_address(config).map_err(UseCaseFailure::without_payload)?;
    if args.client_mcp.is_some() || args.launch.external_epf_wait.is_some() {
        return Err(UseCaseFailure::without_payload(AppError::Validation(
            "launch web opens a browser and takes no client launch options".to_owned(),
        )));
    }
    // У `launch web` адрес один. Ключ, которому нечего выбирать, отвергается, а не
    // принимается молча: молчаливое согласие читалось бы как выбор.
    if args.via.is_some() {
        return Err(UseCaseFailure::without_payload(AppError::Validation(
            "--via selects the address for the thin client; launch web has only the client address"
                .to_owned(),
        )));
    }
    if let Some(interruption) = context.interruption() {
        return Err(UseCaseFailure::without_payload(AppError::Runtime(
            crate::use_cases::interruption::command_interruption_message(context, interruption),
        )));
    }

    let (program, leading) = crate::platform::browser::opener();
    let mut plan_args = leading.clone();
    plan_args.push(url.to_owned());
    // Браузеру идёт настоящий адрес, в отчёт — замаскированный. Считаем один раз:
    // четыре независимых места маскировки разъехались бы.
    let reported_url = mask_url_userinfo(url);
    if args.dry_run {
        let plan_args: Vec<String> = plan_args.iter().map(|arg| mask_url_userinfo(arg)).collect();
        log_live_stage(
            "launch: preview",
            "[Launch] preview only, browser not opened",
        );
        return Ok(LaunchResult {
            ok: true,
            mode: LaunchMode::Web,
            pid: None,
            via: LaunchVia::Web,
            binary: program.clone(),
            platform_resolution: None,
            url: Some(reported_url.clone()),
            provider_dispatched: false,
            plan: Some(LaunchPlan {
                program,
                args: plan_args,
            }),
            message: Some(format!(
                "Previewed веб-клиент at {reported_url}; browser not opened"
            )),
            mcp_readiness: None,
            external_epf_wait: None,
        });
    }

    log_live_stage("launch: web", "[Launch] opening the published infobase");
    let pid = crate::platform::browser::open_url(&program, &leading, url)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    Ok(LaunchResult {
        ok: true,
        mode: LaunchMode::Web,
        pid: Some(pid),
        via: LaunchVia::Web,
        binary: program,
        platform_resolution: None,
        url: Some(reported_url.clone()),
        provider_dispatched: true,
        plan: None,
        message: Some(format!("Opened веб-клиент at {reported_url} (pid {pid})")),
        mcp_readiness: None,
        external_epf_wait: None,
    })
}

/// Клиентский адрес цели. Один текст отказа на оба пути: `launch web` и тонкий клиент по
/// вебу отказывают одинаково, потому что не хватает им одного и того же.
fn client_address(config: &AppConfig) -> Result<&str, AppError> {
    config
        .infobase
        .web
        .as_ref()
        .and_then(|web| web.url.as_deref())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            AppError::Validation(
                "infobase.web.url is not declared: the client address appears after `publish` on a web server or is set by hand in infobase.web.url"
                    .to_owned(),
            )
        })
}

/// Каким адресом открывать базу: то, что попросили, иначе умолчание по виду цели.
///
/// Вид цели берётся объявленным, а не разобранным из строки подключения. Строку прямого
/// шлюза автономной цели раннер пока не использует (#205), поэтому умолчание для неё — веб.
fn resolve_launch_via(
    requested: Option<LaunchVia>,
    client_mode: LaunchClientMode,
    standalone: bool,
) -> Result<LaunchVia, AppError> {
    let default = if standalone {
        LaunchVia::Web
    } else {
        LaunchVia::Connection
    };
    let Some(requested) = requested else {
        return Ok(default);
    };
    if !matches!(client_mode, LaunchClientMode::Thin) {
        return Err(AppError::Validation(
            "--via selects the address for the thin client; the other launch modes have only one address".to_owned(),
        ));
    }
    if requested == LaunchVia::Connection && standalone {
        return Err(AppError::Validation(
            "the direct gate address of a standalone server is not used by the runner yet (#205): the thin client goes by infobase.web.url — use --via web or launch web".to_owned(),
        ));
    }
    Ok(requested)
}

fn client_mcp_launch_shape(mode: ClientMcpMode) -> (LaunchMode, UtilityType, LaunchClientMode) {
    match mode {
        ClientMcpMode::Thin => (LaunchMode::Mcp, UtilityType::V8C, LaunchClientMode::Thin),
        ClientMcpMode::Thick => (LaunchMode::Mcp, UtilityType::V8, LaunchClientMode::Thick),
        ClientMcpMode::Ordinary => (LaunchMode::Mcp, UtilityType::V8, LaunchClientMode::Ordinary),
    }
}

fn effective_launch_options(
    config: &AppConfig,
    args: &LaunchArgs,
) -> Result<LaunchOptions, AppError> {
    let is_client_mcp = matches!(
        args.target,
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. })
    );
    let Some(client_mcp) = args.client_mcp.as_ref() else {
        return if is_client_mcp {
            Err(AppError::Validation(
                "launch mcp requires client_mcp options".to_owned(),
            ))
        } else {
            Ok(args.launch.clone())
        };
    };
    if !is_client_mcp {
        return Err(AppError::Validation(
            "client_mcp options are supported only for launch mcp".to_owned(),
        ));
    }

    let mut launch = args.launch.clone();
    let mut payload = build_client_mcp_payload(client_mcp, config.tools.client_mcp.port);
    if matches!(
        client_mcp.addon,
        Some(ClientMcpAddonRequest::VanessaAutomation)
    ) {
        let va_launch = crate::use_cases::vanessa::prepare_client_mcp_launch(config)?;
        crate::use_cases::vanessa::apply_client_mcp_launch(&mut launch, &mut payload, &va_launch);
    }
    launch.c = Some(payload);
    Ok(launch)
}

fn build_client_mcp_payload(
    options: &ClientMcpOptionsRequest,
    configured_port: Option<u16>,
) -> String {
    let mut payload = match options.config_path.as_deref() {
        Some(path) => format!("runMcp={}", normalize_launch_payload_path(Path::new(path))),
        None => "runMcp".to_owned(),
    };
    if let Some(port) = options.port.or(configured_port) {
        payload.push_str(&format!(";mcpPort={port}"));
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::{execute, platform_resolution};
    use crate::config::model::{
        AppConfig, BuildConfig, EnterpriseToolConfig, PlatformToolConfig, SourceFormat,
        SourceSetConfig, SourceSetPurpose, TestsConfig, ToolExtensionArtifactConfig,
        ToolExtensionConfig, ToolExtensionInput, ToolsConfig,
    };
    use crate::platform::locator::{ResolutionSource, UtilityLocation, UtilityType};
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::{
        ClientMcpMode, ClientMcpOptionsRequest, LaunchRequest, LaunchTargetRequest,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
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
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    #[cfg(unix)]
    fn write_logging_script(path: &Path, args_log: &Path) {
        let staged_log = args_log.with_extension("tmp");
        write_script(
            path,
            &format!(
                "printf '%s\n' \"$@\" > '{}'\nmv '{}' '{}'\nsleep 1",
                staged_log.display(),
                staged_log.display(),
                args_log.display()
            ),
        );
    }

    fn read_args_log(path: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(args) = fs::read_to_string(path) {
                if !args.is_empty() {
                    return args;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for args log '{}'", path.display())
    }

    #[test]
    fn launch_resolution_serializes_all_sources_and_unknown_version_as_null() {
        for (source, expected_source) in [
            (ResolutionSource::Explicit, "explicit"),
            (ResolutionSource::DefaultRoot, "default-root"),
            (ResolutionSource::Path, "path"),
        ] {
            let resolution = platform_resolution(&UtilityLocation {
                utility: UtilityType::V8,
                path: PathBuf::from("/opt/1cv8/bin/1cv8"),
                version: None,
                source,
                installation_root: PathBuf::from("/opt/1cv8"),
            });
            let json = serde_json::to_value(resolution).expect("resolution JSON");

            assert_eq!(json["source"], expected_source);
            assert!(json["version"].is_null());
        }
    }

    fn sample_config(base_path: &Path, work_path: &Path, platform_path: &Path) -> AppConfig {
        AppConfig {
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("."),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(platform_path.to_path_buf()),
                    strict: false,
                    version: None,
                },
                enterprise: EnterpriseToolConfig::default(),
                edt_cli: Default::default(),
                ..Default::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn thin_launch_app_appends_enterprise_additional_keys() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("thin.args.log");
        let platform_dir = dir.path().join("platform");
        write_logging_script(&platform_dir.join("bin").join("1cv8c"), &args_log);

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.enterprise.additional_launch_keys = vec!["/TESTMANAGER".to_owned()];

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: None,
                dry_run: false,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/TESTMANAGER"));
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_launch_plans_the_client_without_spawning_it() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("preview.args.log");
        let platform_dir = dir.path().join("platform");
        write_logging_script(&platform_dir.join("bin").join("1cv8c"), &args_log);

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.enterprise.additional_launch_keys = vec!["/TESTMANAGER".to_owned()];

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: None,
                dry_run: true,
            },
        )
        .expect("preview succeeds");

        assert!(result.ok);
        assert!(!result.provider_dispatched);
        assert!(result.pid.is_none());
        let plan = result.plan.expect("preview plan");
        assert!(
            plan.program.ends_with("platform/bin/1cv8c"),
            "{:?}",
            plan.program
        );
        assert!(plan.args.contains(&"ENTERPRISE".to_owned()));
        assert!(plan.args.contains(&"/TESTMANAGER".to_owned()));
        assert!(
            !args_log.exists(),
            "preview must not dispatch the client process"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_launch_plan_masks_the_infobase_password() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("platform");
        write_script(&platform_dir.join("bin").join("1cv8c"), "sleep 1");

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.infobase.user = Some("Администратор".to_owned());
        config.infobase.password = Some("s3cret".to_owned());

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: None,
                dry_run: true,
            },
        )
        .expect("preview succeeds");

        let plan = result.plan.expect("preview plan");
        let rendered = plan.args.join(" ");
        assert!(
            !rendered.contains("s3cret"),
            "password leaked into the preview: {rendered}"
        );
        assert!(rendered.contains("/N Администратор"));
        assert!(rendered.contains("/P ***"));
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_launch_refuses_options_that_observe_a_running_client() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("platform");
        write_script(&platform_dir.join("bin").join("1cv8c"), "sleep 1");

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.client_mcp.port = Some(9874);

        let error = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest {
                    wait_ready: true,
                    ..ClientMcpOptionsRequest::default()
                }),
                dry_run: true,
            },
        )
        .expect_err("preview cannot wait for a client it never starts");

        assert!(error
            .error
            .to_string()
            .contains("--wait-ready cannot be combined with launch --dry-run"));
    }

    #[cfg(unix)]
    #[test]
    fn designer_launch_app_does_not_append_enterprise_additional_keys() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("designer.args.log");
        let platform_dir = dir.path().join("platform");
        write_logging_script(&platform_dir.join("bin").join("1cv8"), &args_log);

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.enterprise.additional_launch_keys = vec!["/TESTMANAGER".to_owned()];

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::designer(),
                launch: Default::default(),
                client_mcp: None,
                dry_run: false,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("DESIGNER"));
        assert!(args.contains("/DisableStartupDialogs"));
        assert!(!args.contains("/TESTMANAGER"));
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_launch_app_uses_enterprise_binary_and_ordinary_mode_key() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("ordinary.args.log");
        let platform_dir = dir.path().join("platform");
        write_logging_script(&platform_dir.join("bin").join("1cv8"), &args_log);

        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::ordinary_application(),
                launch: Default::default(),
                client_mcp: None,
                dry_run: false,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/RunModeOrdinaryApplication"));
        assert!(args.contains("/DisableStartupDialogs"));
    }

    #[cfg(unix)]
    #[test]
    fn client_mcp_launch_does_not_prepare_configured_tool_extension() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("mcp.args.log");
        let platform_dir = dir.path().join("platform");
        write_logging_script(&platform_dir.join("bin").join("1cv8c"), &args_log);

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.client_mcp.port = Some(9874);
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Artifact(ToolExtensionArtifactConfig {
                path: dir.path().join("client_mcp.cfe"),
            }),
        });

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest::default()),
                dry_run: false,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        assert!(result
            .message
            .as_deref()
            .expect("message")
            .contains("v8-runner build"));
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/C\nrunMcp;mcpPort=9874"));
        assert!(!args.contains("/LoadCfg"));
        assert!(!args.contains("-Extension"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_inconsistent_client_mcp_request_state_before_locating_platform() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("missing-platform");
        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let missing_options = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: None,
                dry_run: false,
            },
        )
        .expect_err("client_mcp options are required");
        assert!(
            missing_options
                .error
                .to_string()
                .contains("launch mcp requires client_mcp options"),
            "{missing_options:?}"
        );

        let unexpected_options = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest::default()),
                dry_run: false,
            },
        )
        .expect_err("client_mcp options are rejected for non-mcp launch");
        assert!(
            unexpected_options
                .error
                .to_string()
                .contains("client_mcp options are supported only for launch mcp"),
            "{unexpected_options:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wait_ready_requires_effective_client_mcp_port_before_locating_platform() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("missing-platform");
        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let error = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                via: None,
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest {
                    wait_ready: true,
                    ..ClientMcpOptionsRequest::default()
                }),
                dry_run: false,
            },
        )
        .expect_err("wait-ready without port should fail before platform lookup");

        assert!(
            error
                .error
                .to_string()
                .contains("launch mcp --wait-ready requires --mcp-port or tools.client_mcp.port"),
            "{error:?}"
        );
    }
}
