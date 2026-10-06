//! Выбор исполнителя перед запуском — один на все операции.
//!
//! План даёт матрица: переопределение — один исполнитель без отката, умолчание — цепочка.
//! Здесь цепочка пробуется по готовности: исполнитель готов, когда его утилита найдена.
//! Первый готовый выбирается, пропущенные до него попадают в квитанцию с причиной; если
//! не готов никто, квитанция называет всех пропущенных, а отказ типизирован.
//!
//! Экспортное семейство пробует готовность глубже (строка соединения, файл базы) и
//! держит свой перебор, но утилиты исполнителя, причины пропуска и отказ берёт отсюда —
//! квитанцию и род отказа отдаёт те же.

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::domain::capability::{
    Operation, Provider, ProviderPlan, ProviderReceipt, SkippedProvider,
};
use crate::platform::locator::{UtilityLocation, UtilityType};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;

/// Исполнитель, выбранный для операции, вместе с найденной утилитой и квитанцией.
///
/// `location` пуста у исполнителя, которому утилита на этой машине не нужна: чужой
/// агент уже поднят, к нему подключаются встроенным клиентом.
#[derive(Debug, Clone)]
pub struct SelectedProvider {
    pub provider: Provider,
    pub location: Option<UtilityLocation>,
    pub receipt: ProviderReceipt,
}

/// Утилиты, которыми исполнитель делает работу: `None` — у исполнителя нет утилиты в
/// этой сборке раннера, пустой список — исполнитель готов без утилит. Есть ли у него
/// адаптер именно этой операции, здесь не решается: это ответ `domain::capability`. Первая
/// в списке становится `location` выбранного исполнителя.
pub(crate) fn utilities_of(provider: Provider, config: &AppConfig) -> Option<Vec<UtilityType>> {
    match provider {
        Provider::Designer => Some(vec![UtilityType::V8]),
        Provider::Ibcmd => Some(vec![UtilityType::Ibcmd]),
        Provider::Webinst => Some(vec![UtilityType::Webinst]),
        // Точку входа агента для файловой и кластерной базы раннер поднимает сам —
        // без платформы на этой машине агента нет. К чужой точке входа подключается
        // встроенный SSH-клиент, утилиты для этого не нужны.
        Provider::Agent => match config.tools.designer_agent.mode() {
            // Шлюз автономного сервера держит сам сервер: утилиты раннеру не нужны.
            _ if config.infobase.standalone.is_some() => Some(Vec::new()),
            Ok(DesignerAgentMode::Attached { .. }) => Some(Vec::new()),
            Ok(DesignerAgentMode::Managed { .. }) | Err(_) => Some(vec![UtilityType::V8]),
        },
        Provider::IbcmdRs => None,
    }
}

/// Первый готовый исполнитель из плана операции.
pub fn select(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    operation: Operation,
) -> Result<SelectedProvider, (AppError, ProviderReceipt)> {
    let plan = config.provider_plan(operation);
    if plan.candidates().is_empty() {
        return Err((
            no_executor(config, operation),
            plan.receipt_for_nobody(Vec::new()),
        ));
    }
    let mut skipped: Vec<SkippedProvider> = Vec::new();

    for provider in plan.candidates() {
        let Some(needed) = utilities_of(provider, config) else {
            skipped.push(no_adapter(provider, operation));
            continue;
        };
        let mut located = Vec::with_capacity(needed.len());
        let mut not_ready = None;
        for utility in needed {
            match utilities.locate(utility) {
                Ok(location) => located.push(location),
                Err(error) => {
                    not_ready = Some(format!("environment is not ready: {error}"));
                    break;
                }
            }
        }
        match not_ready {
            None => {
                let receipt = plan.receipt_for(provider, skipped);
                return Ok(SelectedProvider {
                    provider,
                    location: located.into_iter().next(),
                    receipt,
                });
            }
            Some(reason) => skipped.push(SkippedProvider { provider, reason }),
        }
    }

    let error = nobody_ready(config, &plan, &skipped);
    Err((error, plan.receipt_for_nobody(skipped)))
}

/// Отказ, когда у операции на цели этого вида нет ни одного исполнителя.
pub(crate) fn no_executor(config: &AppConfig, operation: Operation) -> AppError {
    AppError::capability(format!(
        "no executor implements {operation} on a {} target",
        config.target_kind().as_str()
    ))
}

/// Пропуск исполнителя, у которого в этой сборке нет адаптера для операции.
pub(crate) fn no_adapter(provider: Provider, operation: Operation) -> SkippedProvider {
    SkippedProvider {
        provider,
        reason: format!(
            "no adapter for {provider} is implemented for {operation} in this build of the runner"
        ),
    }
}

/// Отказ, когда не готов никто: перечень пропущенных с причинами. Род — среда, если хоть
/// у одного кандидата плана утилита в этой сборке есть, иначе — возможность.
pub(crate) fn nobody_ready(
    config: &AppConfig,
    plan: &ProviderPlan,
    skipped: &[SkippedProvider],
) -> AppError {
    let reason = skipped
        .iter()
        .map(|entry| format!("{}: {}", entry.provider.as_str(), entry.reason))
        .collect::<Vec<_>>()
        .join("; ");
    let had_an_adapter = plan
        .candidates()
        .into_iter()
        .any(|provider| utilities_of(provider, config).is_some());
    if had_an_adapter {
        AppError::EnvironmentUnavailable(reason)
    } else {
        AppError::capability(reason)
    }
}

/// Форма ответа, которая несёт квитанцию о выборе исполнителя.
pub trait CarriesReceipt {
    fn attach_receipt(&mut self, receipt: ProviderReceipt);
    fn receipt_mut(&mut self) -> Option<&mut ProviderReceipt>;
}

macro_rules! carries_receipt {
    ($($ty:ty),* $(,)?) => {
        $(impl CarriesReceipt for $ty {
            fn attach_receipt(&mut self, receipt: ProviderReceipt) {
                self.provider = Some(receipt);
            }

            fn receipt_mut(&mut self) -> Option<&mut ProviderReceipt> {
                self.provider.as_mut()
            }
        })*
    };
}

carries_receipt!(
    crate::domain::init::InitResult,
    crate::domain::build::BuildResult,
    crate::domain::dump::DumpResult,
    crate::domain::extensions::ExtensionsResult,
    crate::domain::extensions::ExtensionInventoryResult,
    crate::domain::syntax::SyntaxCheckResult,
    crate::domain::load::LoadResult,
    crate::domain::artifacts::ArtifactsResult,
    crate::domain::publish::PublishResult,
    crate::domain::infobase_export::ExportConfigurationPackageResult,
    crate::domain::infobase_export::ExportInfobaseSnapshotResult,
    crate::domain::infobase_export::RestoreInfobaseSnapshotResult,
);

/// Кладёт квитанцию и в успешный ответ, и в типизированный отказ с полезной нагрузкой:
/// вызывающий видит, кто исполнял, независимо от исхода.
pub fn attach<T: CarriesReceipt>(
    mut outcome: crate::use_cases::result::UseCaseResult<T>,
    receipt: &ProviderReceipt,
) -> crate::use_cases::result::UseCaseResult<T> {
    if let Some(payload) = crate::use_cases::result::payload_mut(&mut outcome) {
        payload.attach_receipt(receipt.clone());
    }
    outcome
}

/// Называет в квитанции точку входа сессии агента, если команда её открывала, — и в
/// успешном ответе, и в отказе с полезной нагрузкой. Адрес берётся только из отметки,
/// которую оставило подключение (`agent_session::connect`): превью, процесс платформы
/// и отказ до подключения сессии не открывали, и поля у них нет.
pub(crate) fn stamp_session<T: CarriesReceipt>(
    mut outcome: crate::use_cases::result::UseCaseResult<T>,
    context: &crate::use_cases::context::ExecutionContext,
) -> crate::use_cases::result::UseCaseResult<T> {
    let Some(endpoint) = context.opened_session() else {
        return outcome;
    };
    if let Some(receipt) =
        crate::use_cases::result::payload_mut(&mut outcome).and_then(CarriesReceipt::receipt_mut)
    {
        receipt.endpoint = Some(endpoint);
    }
    outcome
}
