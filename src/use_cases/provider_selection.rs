//! Выбор исполнителя перед запуском — один на все операции.
//!
//! План даёт матрица: переопределение — один исполнитель без отката, умолчание — цепочка.
//! Здесь цепочка пробуется по готовности: исполнитель готов, когда его утилита найдена.
//! Первый готовый выбирается, пропущенные до него попадают в квитанцию с причиной; если
//! не готов никто, квитанция называет всех пропущенных, а отказ типизирован.
//!
//! Экспортное семейство пробует готовность глубже (строка соединения, файл базы) и
//! держит свой перебор, но квитанцию отдаёт ту же.

use crate::config::model::AppConfig;
use crate::domain::capability::{Operation, Provider, ProviderReceipt, SkippedProvider};
use crate::platform::locator::{UtilityLocation, UtilityType};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;

/// Исполнитель, выбранный для операции, вместе с найденной утилитой и квитанцией.
#[derive(Debug, Clone)]
pub struct SelectedProvider {
    pub provider: Provider,
    pub location: UtilityLocation,
    pub receipt: ProviderReceipt,
}

/// Утилита, которой исполнитель делает работу; `None` — адаптера в этой сборке нет.
fn utility_of(provider: Provider) -> Option<UtilityType> {
    match provider {
        Provider::Designer => Some(UtilityType::V8),
        Provider::Ibcmd => Some(UtilityType::Ibcmd),
        Provider::Webinst => Some(UtilityType::Webinst),
        Provider::Agent | Provider::IbcmdRs => None,
    }
}

/// Первый готовый исполнитель из плана операции.
pub fn select(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    operation: Operation,
) -> Result<SelectedProvider, (AppError, ProviderReceipt)> {
    let plan = config.provider_plan(operation);
    let mut skipped: Vec<SkippedProvider> = Vec::new();
    let mut had_an_adapter = false;

    for provider in plan.candidates() {
        let Some(utility) = utility_of(provider) else {
            skipped.push(SkippedProvider {
                provider,
                reason: format!(
                    "no adapter for {provider} is implemented for {operation} in this build of the runner"
                ),
            });
            continue;
        };
        had_an_adapter = true;
        match utilities.locate(utility) {
            Ok(location) => {
                let receipt = plan.receipt_for(provider, skipped);
                return Ok(SelectedProvider {
                    provider,
                    location,
                    receipt,
                });
            }
            Err(error) => skipped.push(SkippedProvider {
                provider,
                reason: format!("environment is not ready: {error}"),
            }),
        }
    }

    let reason = skipped
        .iter()
        .map(|entry| format!("{}: {}", entry.provider.as_str(), entry.reason))
        .collect::<Vec<_>>()
        .join("; ");
    let receipt = plan.receipt_for_nobody(skipped);
    let error = if had_an_adapter {
        AppError::EnvironmentUnavailable(reason)
    } else {
        AppError::CapabilityUnavailable(reason)
    };
    Err((error, receipt))
}

/// Форма ответа, которая несёт квитанцию о выборе исполнителя.
pub trait CarriesReceipt {
    fn attach_receipt(&mut self, receipt: ProviderReceipt);
}

macro_rules! carries_receipt {
    ($($ty:ty),* $(,)?) => {
        $(impl CarriesReceipt for $ty {
            fn attach_receipt(&mut self, receipt: ProviderReceipt) {
                self.provider = Some(receipt);
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
);

/// Кладёт квитанцию и в успешный ответ, и в типизированный отказ с полезной нагрузкой:
/// вызывающий видит, кто исполнял, независимо от исхода.
pub fn attach<T: CarriesReceipt>(
    outcome: crate::use_cases::result::UseCaseResult<T>,
    receipt: &ProviderReceipt,
) -> crate::use_cases::result::UseCaseResult<T> {
    match outcome {
        Ok(mut result) => {
            result.attach_receipt(receipt.clone());
            Ok(result)
        }
        Err(mut failure) => {
            if let Some(payload) = failure.payload.as_mut() {
                payload.attach_receipt(receipt.clone());
            }
            Err(failure)
        }
    }
}
