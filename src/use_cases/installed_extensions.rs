//! Состав расширений базы для обходов по составу базы: `pull --all` и `download` без набора.
//!
//! Вызов выбирает исполнитель команды (замер #187: у каждого свой), ответ разбирается по
//! структуре, а не по тексту сообщений.

use std::path::Path;

use crate::config::model::AppConfig;
use crate::domain::capability::{Operation, Provider};
use crate::platform::extension_inventory::{
    is_extension_identifier, parse_extension_inventory, parse_extension_name_list,
};
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::dump_config::helpers::{
    build_designer_dsl, build_ibcmd_dsl, ensure_success_of, map_ibcmd_error,
};
use crate::use_cases::extension_agent::ExtensionAgent;
use crate::use_cases::progress::log_live_stage;

/// Имена расширений, установленных в базе, — вызовом исполнителя команды (замер #187).
/// Читатель один: им спрашивают базу `pull --all` и `download` без набора.
///
/// Имя, которое не является идентификатором 1С, — неверный вывод: оно станет каталогом и
/// доводом платформы, и угадывать его нельзя.
pub(crate) fn read_installed_extensions(
    context: &ExecutionContext,
    config: &AppConfig,
    operation: Operation,
    provider: Provider,
    binary: Option<&Path>,
    utilities: &PlatformUtilities,
) -> Result<Vec<String>, AppError> {
    // Подпись и журнал `pull --all` — прежние (`[Pull]`, `dump-extensions-list.log`); у
    // `download` — те же по своему имени.
    let command = context.command().as_str();
    let mut label = command.to_owned();
    if let Some(first) = label.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    log_live_stage(
        &format!("{command}: extensions"),
        &format!("[{label}] reading the extensions installed in the infobase"),
    );
    let names = match (provider, binary) {
        (Provider::Designer, Some(binary)) => {
            let dsl = build_designer_dsl(
                context,
                config,
                binary,
                utilities.runner_for(UtilityType::V8),
                "extensions",
                "list",
            )?;
            let listed = dsl
                .dump_db_cfg_list_all_extensions()
                .map_err(AppError::from)?;
            ensure_listed(&listed)?;
            let Some(out) = listed.platform_log.as_deref() else {
                return Err(AppError::InvalidOutput(format!(
                    "the Designer extension list was not read: {}",
                    listed
                        .platform_log_read_error
                        .as_deref()
                        .unwrap_or("no /Out log")
                )));
            };
            parse_extension_name_list(out).map_err(AppError::InvalidOutput)?
        }
        (Provider::Ibcmd, Some(binary)) => {
            let dsl = build_ibcmd_dsl(
                context,
                config,
                binary,
                utilities.runner_for(UtilityType::Ibcmd),
            )?;
            let listed = dsl.infobase_extension_list().map_err(map_ibcmd_error)?;
            ensure_listed(&listed)?;
            parse_extension_inventory(&listed.process.stdout)
                .map_err(AppError::InvalidOutput)?
                .into_iter()
                .map(|extension| extension.name)
                .collect()
        }
        (Provider::Agent, binary) => {
            let mut agent = ExtensionAgent::open(context, config, binary)?;
            let inventory = agent.inventory(None);
            agent.close();
            inventory?
                .into_iter()
                .map(|extension| extension.name)
                .collect()
        }
        // Без утилиты исполнитель списка не прочтёт, а прочие исполнители выгрузку не
        // делают: тот же отказ, что у выгрузки без адаптера.
        (provider @ (Provider::Designer | Provider::Ibcmd), None)
        | (provider @ (Provider::IbcmdRs | Provider::Webinst), _) => {
            return Err(crate::use_cases::unimplemented_provider(
                operation, provider,
            ))
        }
    };
    if let Some(name) = names.iter().find(|name| !is_extension_identifier(name)) {
        return Err(AppError::InvalidOutput(format!(
            "the infobase lists an extension whose name is not an identifier: {name:?}"
        )));
    }
    Ok(names)
}

/// Список прочитан, только когда процесс завершился удачно.
fn ensure_listed(result: &PlatformCommandResult) -> Result<(), AppError> {
    ensure_success_of(
        "list extensions of",
        "infobase",
        "the configured infobase",
        result,
    )
}
