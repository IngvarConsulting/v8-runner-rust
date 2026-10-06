//! `download` без набора: пакет каждого набора конфигурации по составу базы.
//!
//! Команда выбирает исполнителя выгрузки, спрашивает им базу, какие расширения в ней есть
//! (тем же читателем, что `pull --all`), и обходит пакеты конфигурации порядком
//! [`SourceSetInventory::configuration_packages`]. Каждый пакет выгружается тем же сценарием,
//! что `download <SET>`, в [`package_in_directory`]; набор расширения, которого в базе нет,
//! не выгружается и называется в `not_installed`. Отказ набора останавливает обход.

use std::collections::HashSet;
use std::time::Instant;

use crate::config::model::{AppConfig, SourceSetConfig};
use crate::domain::capability::Operation;
use crate::domain::infobase_export::{
    ConfigurationSubject, DownloadAllResult, ExportConfigurationPackageRequest,
};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::extension_identity::extension_name_key;
use crate::use_cases::request::DownloadAllRequest;
use crate::use_cases::result::{stamp_dispatch, UseCaseError, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::{package_in_directory, SourceSetInventory};

use super::{InfobaseTransferIntent, PreparedTransferProvider};

/// Единственный вход: квитанция называет точку входа сессии агента, а `provider_dispatched`
/// ставит отметка работы команды — чтение состава базы уже работа.
#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_configuration_export_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &DownloadAllRequest,
) -> UseCaseResult<DownloadAllResult> {
    stamp_dispatch(
        crate::use_cases::provider_selection::stamp_session(
            run_all(context, config, request),
            context,
        ),
        context.work(),
    )
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
fn run_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &DownloadAllRequest,
) -> UseCaseResult<DownloadAllResult> {
    let started = Instant::now();
    let mut result = DownloadAllResult {
        provider: None,
        ok: false,
        provider_dispatched: false,
        output: request.output_directory.clone(),
        not_installed: Vec::new(),
        if_installed: Vec::new(),
        sets: Vec::new(),
        duration_ms: 0,
        message: None,
    };
    let packages = SourceSetInventory::new(config).configuration_packages();
    // Обойти нечего — отказ до выбора исполнителя: пустой обход ответил бы успехом, ничего
    // не выгрузив.
    if packages.is_empty() {
        let error = AppError::Validation(
            "download without <SET> takes the configuration and extension source-sets, and the project has none".to_owned(),
        );
        return Err(fail(error.into(), result, started));
    }
    let prepared = match super::select_provider(
        context,
        config,
        InfobaseTransferIntent::Configuration {
            state: request.state,
        },
    ) {
        Ok(prepared) => prepared,
        Err((error, receipt)) => {
            result.provider = Some(receipt);
            return Err(fail(super::infobase_use_case_error(error), result, started));
        }
    };
    result.provider = Some(prepared.receipt.clone());
    let walker = Walker {
        context,
        config,
        request,
        prepared: &prepared,
    };

    if request.dry_run {
        return walker.preview(&packages, result, started);
    }

    let installed = match crate::use_cases::dump_config::read_installed_extensions(
        context,
        config,
        Operation::ConfigurationExport,
        prepared.provider,
        prepared.executable.as_deref(),
        &PlatformUtilities::from_config(config),
    ) {
        Ok(installed) => installed
            .iter()
            .map(|name| extension_name_key(name))
            .collect::<HashSet<_>>(),
        Err(error) => {
            result.message = Some(error.to_string());
            return Err(UseCaseFailure::after_possible_work(
                super::infobase_use_case_error(error),
                context.work(),
                || {
                    result.duration_ms = started.elapsed().as_millis() as u64;
                    result
                },
            ));
        }
    };
    let (walked, not_installed): (Vec<_>, Vec<_>) =
        packages.into_iter().partition(|(_, extension)| {
            extension.is_none_or(|name| installed.contains(&extension_name_key(name)))
        });
    result.not_installed = not_installed
        .into_iter()
        .map(|(source_set, _)| source_set.name.clone())
        .collect();
    for (source_set, extension) in walked {
        if let Err(error) = walker.download(source_set, extension, &mut result) {
            return Err(fail(error, result, started));
        }
    }
    result.ok = true;
    result.duration_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}

/// Отказ: ответ несёт выгруженное до него и называет причину.
fn fail(
    error: UseCaseError,
    mut result: DownloadAllResult,
    started: Instant,
) -> UseCaseFailure<DownloadAllResult> {
    result.duration_ms = started.elapsed().as_millis() as u64;
    result.message = Some(error.to_string());
    UseCaseFailure::with_payload(error, result)
}

/// Общее для каждой выгрузки обхода: сценарий, запрос и выбранный исполнитель.
struct Walker<'a> {
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    request: &'a DownloadAllRequest,
    prepared: &'a PreparedTransferProvider,
}

impl Walker<'_> {
    /// Превью платформу не запускает и состава базы не знает: превью выгрузки получают
    /// наборы конфигурации, которые прогон выгрузит при любом составе, а наборы расширений
    /// названы в `if_installed`.
    #[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
    fn preview(
        &self,
        packages: &[(&SourceSetConfig, Option<&str>)],
        mut result: DownloadAllResult,
        started: Instant,
    ) -> UseCaseResult<DownloadAllResult> {
        for (source_set, extension) in packages {
            if extension.is_some() {
                result.if_installed.push(source_set.name.clone());
                continue;
            }
            let set_request = self.set_request(source_set, None);
            match super::preview_configuration_export(
                self.context,
                self.config,
                &set_request,
                self.prepared,
            ) {
                Ok(previewed) => result.sets.push(previewed),
                Err(failure) => {
                    result.sets.extend(failure.payload);
                    return Err(fail(failure.error, result, started));
                }
            }
        }
        result.message = Some(format!(
            "would read the extensions installed in the infobase via {}, download each extension set of the project the infobase has into '{}' and name the others as not installed; nothing read, nothing written",
            self.prepared.provider,
            self.request.output_directory.display()
        ));
        result.ok = true;
        result.duration_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }

    /// Выгружает один пакет сценарием `download <SET>` и кладёт его ответ в обход.
    fn download(
        &self,
        source_set: &SourceSetConfig,
        extension: Option<&str>,
        result: &mut DownloadAllResult,
    ) -> Result<(), UseCaseError> {
        let set_request = self.set_request(source_set, extension);
        match super::execute_configuration_export(
            self.context,
            self.config,
            &set_request,
            self.prepared,
        ) {
            Ok(downloaded) => {
                result.sets.push(downloaded);
                Ok(())
            }
            Err(failure) => {
                result.sets.extend(failure.payload);
                Err(failure.error)
            }
        }
    }

    fn set_request(
        &self,
        source_set: &SourceSetConfig,
        extension: Option<&str>,
    ) -> ExportConfigurationPackageRequest {
        ExportConfigurationPackageRequest {
            state: self.request.state,
            subject: ConfigurationSubject::of_extension(extension),
            output: package_in_directory(&self.request.output_directory, source_set),
        }
    }
}
