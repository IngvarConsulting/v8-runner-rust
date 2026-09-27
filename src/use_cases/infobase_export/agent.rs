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
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    argument, collect_file, make_output_dir, run_command, run_id, stage_file, tidy, tidy_run_path,
    transcript_log, with_session, Exchange,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::interruption::CommandFailure;
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
) -> Result<PlatformCommandResult, CommandFailure> {
    if state == ConfigurationState::Database {
        return Err(CommandFailure::without_deferral(AppError::capability(
            "the agent exports only the working configuration: it has no command for the database configuration; use providers.infobase.configuration.export: designer or ibcmd".to_owned(),
        )));
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
        transcript_log(config, "infobase-export").map_err(CommandFailure::without_deferral)?,
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
) -> Result<PlatformCommandResult, CommandFailure> {
    with_session(
        context,
        config,
        v8,
        transcript_log(config, "infobase-dump").map_err(CommandFailure::without_deferral)?,
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
) -> Result<PlatformCommandResult, CommandFailure> {
    with_session(
        context,
        config,
        v8,
        transcript_log(config, "infobase-restore").map_err(CommandFailure::without_deferral)?,
        |handle, wait, exchange| {
            let relative = format!("restore/{}.dt", run_id());
            stage_file(handle, exchange, &relative, source_file)?;
            log_live_stage(
                "infobase restore: agent",
                "[агент] restoring infobase snapshot",
            );
            // Загрузка снимка подменяет базу целиком: фаза критическая, её не бросают
            // на полпути (`DEC.2026-04-20.A-MUTATING-CRITICAL-PHASE-IS-NOT-HARD-KILLED`).
            let outcome = run_command(
                handle,
                &format!("infobase-tools restore-ib --file={}", argument(&relative)),
                &wait.critical(),
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
