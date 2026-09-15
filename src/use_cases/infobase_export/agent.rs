//! Экспортное семейство через агентский shell: `dump-cfg`, `dump-ib`, `restore-ib`.
//!
//! Файлы агент пишет только в настоящий подкаталог своего каталога пользователя —
//! через символическую ссылку он их не видит (замер 15.09.2026), поэтому результат
//! сначала появляется там и лишь потом забирается каналом обмена в стадию
//! публикации семейства.

use std::path::Path;

use crate::config::model::AppConfig;
use crate::domain::infobase_export::ConfigurationState;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    argument, collect_file, connect, make_output_dir, platform_result, run_command, run_id,
    stage_file, tidy, tidy_run_path, transcript_log, wait_policy, AgentHandle, Exchange,
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
        |handle, wait, exchange| {
            let out = format!("export/{}", run_id());
            make_output_dir(handle, exchange, &out)?;
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
                .map(|_| collect_file(handle, exchange, &relative, staging_path));
            tidy(handle, exchange, &out);
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
        |handle, wait, exchange| {
            let out = format!("export/{}", run_id());
            make_output_dir(handle, exchange, &out)?;
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
                .map(|_| collect_file(handle, exchange, &relative, staging_path));
            tidy(handle, exchange, &out);
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
        |handle, wait, exchange| {
            let relative = format!("restore/{}.dt", run_id());
            stage_file(handle, exchange, &relative, source_file)?;
            log_live_stage(
                "infobase restore: agent",
                "[агент] restoring infobase snapshot",
            );
            let outcome = run_command(
                handle,
                &format!("infobase-tools restore-ib --file={}", argument(&relative)),
                wait,
            );
            // После загрузки сессии уже нет: убрать копию DT по SFTP не выйдет, а из
            // каталога — можно.
            if let Exchange::Dir(user_dir) = exchange {
                tidy_run_path(user_dir, &relative);
            }
            Ok(outcome?.transcript())
        },
    )
}

/// Одна сессия на операцию: открыть, выполнить, закрыть — и при отказе тоже.
fn with_session(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    log_name: &str,
    work: impl FnOnce(
        &mut AgentHandle,
        &crate::platform::agent::WaitPolicy,
        &Exchange,
    ) -> Result<String, AppError>,
) -> Result<PlatformCommandResult, AppError> {
    let wait = wait_policy(context);
    let log = transcript_log(config, log_name)?;
    let mut utilities = PlatformUtilities::from_config(config);
    let mut handle = connect(config, &mut utilities, v8, log.clone(), &wait)?;
    let outcome = handle
        .exchange(config)
        .and_then(|exchange| work(&mut handle, &wait, &exchange));
    handle.finish(&wait);
    outcome.map(|transcript| platform_result(transcript, log))
}
