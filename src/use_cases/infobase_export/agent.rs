//! Экспортное семейство через агентский shell: `dump-cfg`, `dump-ib`, `restore-ib`.
//!
//! Файлы агент пишет только в настоящий подкаталог своего каталога пользователя —
//! через символическую ссылку он их не видит (замер 15.09.2026), поэтому результат
//! сначала появляется там и лишь потом переносится в стадию публикации семейства.

use std::path::Path;

use crate::config::model::AppConfig;
use crate::domain::infobase_export::ConfigurationState;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::move_file;
use crate::use_cases::agent_session::{
    argument, connect, expose_file, output_dir, platform_result, run_command, run_id,
    transcript_log, wait_policy,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::progress::log_live_stage;

/// `config dump-cfg --file=… [--extension=…]` — только рабочая конфигурация: у агента
/// нет команды для конфигурации базы данных.
pub(super) fn export_configuration(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    state: ConfigurationState,
    extension: Option<&str>,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    if state == ConfigurationState::Database {
        return Err(AppError::CapabilityUnavailable(
            "the agent exports only the working configuration: it has no command for the database configuration; use providers.infobase.configuration.export: designer or ibcmd".to_owned(),
        ));
    }
    let name = match extension {
        Some(extension) => format!("{extension}.cfe"),
        None => "main.cf".to_owned(),
    };
    let mut command = String::from("config dump-cfg");
    with_session(
        context,
        config,
        v8,
        "infobase-export",
        |handle, wait, user_dir| {
            let out = format!("export/{}", run_id());
            output_dir(user_dir, &out)?;
            let relative = format!("{out}/{name}");
            command.push_str(&format!(" --file={}", argument(&relative)));
            if let Some(extension) = extension {
                command.push_str(&format!(" --extension={}", argument(extension)));
            }
            log_live_stage(
                "infobase export: agent",
                "[агент] exporting configuration package",
            );
            let reply = run_command(handle, &command, wait);
            let staged = reply
                .as_ref()
                .ok()
                .map(|_| take_produced(&user_dir.join(&relative), staging_path));
            let _ = std::fs::remove_dir_all(user_dir.join(&out));
            let reply = reply?;
            staged.transpose()?;
            Ok(reply.transcript())
        },
    )
}

/// `infobase-tools dump-ib --file=…`.
pub(super) fn export_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    with_session(
        context,
        config,
        v8,
        "infobase-dump",
        |handle, wait, user_dir| {
            let out = format!("export/{}", run_id());
            output_dir(user_dir, &out)?;
            let relative = format!("{out}/infobase.dt");
            log_live_stage(
                "infobase dump: agent",
                "[агент] exporting infobase snapshot",
            );
            let reply = run_command(
                handle,
                &format!("infobase-tools dump-ib --file={}", argument(&relative)),
                wait,
            );
            let staged = reply
                .as_ref()
                .ok()
                .map(|_| take_produced(&user_dir.join(&relative), staging_path));
            let _ = std::fs::remove_dir_all(user_dir.join(&out));
            let reply = reply?;
            staged.transpose()?;
            Ok(reply.transcript())
        },
    )
}

/// `infobase-tools restore-ib --file=…`: после загрузки агент завершает сеанс и рвёт
/// соединение сам (документация 4.7.7.6); итоговое сообщение ждётся до разрыва.
pub(super) fn restore_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    source_file: &Path,
) -> Result<PlatformCommandResult, AppError> {
    with_session(
        context,
        config,
        v8,
        "infobase-restore",
        |handle, wait, user_dir| {
            let relative = format!("restore/{}.dt", run_id());
            expose_file(user_dir, &relative, source_file)?;
            log_live_stage(
                "infobase restore: agent",
                "[агент] restoring infobase snapshot",
            );
            let outcome = run_command(
                handle,
                &format!("infobase-tools restore-ib --file={}", argument(&relative)),
                wait,
            );
            let _ = std::fs::remove_file(user_dir.join(&relative));
            Ok(outcome?.transcript())
        },
    )
}

fn take_produced(produced: &Path, staging_path: &Path) -> Result<(), AppError> {
    if !produced.is_file() {
        return Err(AppError::Platform(format!(
            "agent reported success but wrote no file at '{}'",
            produced.display()
        )));
    }
    move_file(produced, staging_path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to stage the agent's file '{}': {error}",
            produced.display()
        ))
    })
}

/// Одна сессия на операцию: открыть, выполнить, закрыть — и при отказе тоже.
fn with_session(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    log_name: &str,
    work: impl FnOnce(
        &mut crate::use_cases::agent_session::AgentHandle,
        &crate::platform::agent::WaitPolicy,
        &Path,
    ) -> Result<String, AppError>,
) -> Result<PlatformCommandResult, AppError> {
    let wait = wait_policy(context);
    let log = transcript_log(config, log_name)?;
    let mut utilities = PlatformUtilities::from_config(config);
    let mut handle = connect(config, &mut utilities, v8, log.clone(), &wait)?;
    let outcome = handle
        .user_dir(config)
        .and_then(|user_dir| work(&mut handle, &wait, &user_dir));
    handle.finish(&wait);
    outcome.map(|transcript| platform_result(transcript, log))
}
