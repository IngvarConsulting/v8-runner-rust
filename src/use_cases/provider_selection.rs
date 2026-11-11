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
use crate::platform::locator::{
    LocatorError, PlatformReleaseMissing, UtilityLocation, UtilityType,
};
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
    select_from(
        config,
        utilities,
        operation,
        config.provider_plan(operation),
    )
}

/// Первый готовый исполнитель операции данного объёма (`AppConfig::provider_plan_sized`):
/// квитанция и в удаче, и в отказе называет причину порядка, если порядок выбран по объёму.
///
/// Дальше по цепочке выбор идёт только по готовности, как у [`select`]: исполнитель, утилита
/// которого найдена, выбран, и его отказ при исполнении на следующего не переводит.
pub fn select_sized(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    operation: Operation,
    size: crate::domain::capability::OperationSize,
) -> Result<SelectedProvider, (AppError, ProviderReceipt)> {
    let (plan, reason) = config.provider_plan_sized(operation, size);
    match select_from(config, utilities, operation, plan) {
        Ok(mut selected) => {
            selected.receipt = selected.receipt.with_reason(reason);
            Ok(selected)
        }
        Err((error, receipt)) => Err((error, receipt.with_reason(reason))),
    }
}

/// Первый готовый исполнитель из данного плана: так операция сужает план, когда часть её
/// работы умеет только один исполнитель (`make` внешних обработок — Конфигуратор).
pub fn select_from(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    operation: Operation,
    plan: ProviderPlan,
) -> Result<SelectedProvider, (AppError, ProviderReceipt)> {
    if plan.candidates().is_empty() {
        return Err((
            no_executor(config, operation),
            plan.receipt_for_nobody(Vec::new()),
        ));
    }
    let mut skipped: Vec<SkippedProvider> = Vec::new();
    let mut release = ReleaseRefusal::default();

    for provider in plan.candidates() {
        if let Some(skip) = without_a_way(config, operation, provider) {
            skipped.push(skip);
            continue;
        }
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
                    not_ready = Some(release.not_ready(error));
                    break;
                }
            }
        }
        match not_ready {
            None => {
                release_for_another_executor(config, operation, provider);
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

    let error = nobody_ready(config, &plan, &skipped, release);
    Err((error, plan.receipt_for_nobody(skipped)))
}

/// Выбран исполнитель операции, и это не агент: живые агенты этой базы, оставшиеся
/// подключёнными после прошлой команды, отключаются от неё, чтобы он не застал базу занятой
/// (#644, `INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`). Единственное место
/// этого решения у всех выборщиков исполнителя; операции, которые базу проекта не открывают,
/// агента не трогают.
pub(crate) fn release_for_another_executor(
    config: &AppConfig,
    operation: Operation,
    provider: Provider,
) {
    if provider != Provider::Agent && operation.opens_the_infobase() {
        crate::use_cases::agent_session::release_live_connections(config, operation.as_str(), None);
    }
}

/// Отказ выпуска формата исходников, замеченный при переборе исполнителей. Выпуск закреплён
/// за всеми утилитами платформы, поэтому, если не готов никто, отказом становится он сам, а не
/// пересказ в перечне пропущенных: его код и опись нужны вызывающему
/// (`INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED`).
#[derive(Debug, Default)]
pub(crate) struct ReleaseRefusal(Option<Box<PlatformReleaseMissing>>);

impl ReleaseRefusal {
    /// Причина пропуска исполнителя по ошибке поиска его утилиты; отказ выпуска запоминается.
    pub(crate) fn not_ready(&mut self, error: LocatorError) -> String {
        let reason = format!("environment is not ready: {error}");
        if let LocatorError::PlatformReleaseMissing(missing) = error {
            self.0.get_or_insert(missing);
        }
        reason
    }
}

/// Отказ, когда у операции на цели этого вида нет ни одного исполнителя. У автономного
/// сервера строка бывает пуста потому, что исполнителям её не объявлен путь: тогда отказ
/// — ошибка конфигурации и называет, что объявить.
pub(crate) fn no_executor(config: &AppConfig, operation: Operation) -> AppError {
    if let Some(undeclared) = config.undeclared_way(operation) {
        return AppError::Validation(undeclared);
    }
    AppError::capability(format!(
        "no executor implements {operation} on a {} target",
        config.target_kind().as_str()
    ))
}

/// Пропуск исполнителя, которому не объявлен путь к автономному серверу. В цепочку
/// умолчаний такой исполнитель не попадает вовсе; здесь его встречает только ключ
/// `providers.*`, назначивший его, — и не подменяется умолчанием.
pub(crate) fn without_a_way(
    config: &AppConfig,
    operation: Operation,
    provider: Provider,
) -> Option<SkippedProvider> {
    config
        .missing_way(operation, provider)
        .map(|way| SkippedProvider {
            provider,
            reason: way.undeclared(provider),
        })
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
/// у одного кандидата плана утилита в этой сборке есть, иначе — возможность. Отказ выпуска
/// формата исходников приходит как есть.
pub(crate) fn nobody_ready(
    config: &AppConfig,
    plan: &ProviderPlan,
    skipped: &[SkippedProvider],
    release: ReleaseRefusal,
) -> AppError {
    if let Some(missing) = release.0 {
        return AppError::PlatformLocator(LocatorError::PlatformReleaseMissing(missing));
    }
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
    crate::domain::apply::ApplyResult,
    crate::domain::reset::ResetResult,
    crate::domain::dump::DumpResult,
    crate::domain::dump::PullAllResult,
    crate::domain::extensions::ExtensionsResult,
    crate::domain::extensions::ExtensionInventoryResult,
    crate::domain::syntax::SyntaxCheckResult,
    crate::domain::load::LoadResult,
    crate::domain::artifacts::ArtifactsResult,
    crate::domain::convert::ConvertResult,
    crate::domain::publish::PublishResult,
    crate::domain::infobase_export::ExportConfigurationPackageResult,
    crate::domain::infobase_export::DownloadAllResult,
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

#[cfg(test)]
mod tests {
    use super::select;
    use crate::config::model::{
        AppConfig, InfobaseConfig, PlatformToolConfig, SourceFormat, TestsConfig, ToolsConfig,
    };
    use crate::domain::capability::Operation;
    use crate::domain::platforms::SourceFormatInfo;
    use crate::platform::locator::{
        Locator, LocatorError, PlatformResolutionPolicy, PlatformVersionRequirement,
        RequiredRelease, SourceFormatRelease, UtilityType,
    };
    use crate::platform::utilities::PlatformUtilities;
    use crate::support::error::AppError;
    use crate::use_cases::result::{EnvironmentReason, UseCaseError, UseCaseErrorKind};
    use std::path::PathBuf;

    /// Ни один исполнитель цепочки не готов, потому что при `sources` нет выпуска формата
    /// исходников: отказ выбора — сам отказ выпуска с его описью, а не пересказ в перечне
    /// пропущенных `environment_unavailable`. Сторож против возврата к пересказу.
    #[test]
    fn a_missing_release_of_the_sources_format_reaches_the_caller_typed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("roots");
        for utility in [UtilityType::V8, UtilityType::Ibcmd] {
            let executable = root.join("8.5.4.1306").join(utility.executable_name());
            std::fs::create_dir_all(executable.parent().expect("parent")).expect("dirs");
            std::fs::write(&executable, "#!/bin/sh\nexit 0\n").expect("utility");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod");
            }
        }
        let locator = Locator::with_search_roots(
            None,
            None,
            PlatformResolutionPolicy::Lenient,
            None,
            None,
            vec![root],
            Vec::new(),
            Vec::new(),
        )
        .with_source_format(SourceFormatRelease::Required(RequiredRelease {
            releases: PlatformVersionRequirement::parse("8.5.1")
                .into_iter()
                .collect(),
            source_format: SourceFormatInfo {
                version: Some("2.21".to_owned()),
                file: PathBuf::from("/project/src/Configuration.xml"),
                release: Some("8.5.1".to_owned()),
            },
            unreadable: None,
        }));
        let mut utilities = PlatformUtilities::with_locator(locator);
        let config = AppConfig {
            local_path: dir.path().join(".local"),
            base_path: dir.path().to_path_buf(),
            work_path: dir.path().join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            working_copy: None,
            project_file: crate::config::model::default_project_file(),
            source_sets: Vec::new(),
            tools: ToolsConfig {
                platform: PlatformToolConfig::default(),
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        };

        let (error, receipt) =
            select(&config, &mut utilities, Operation::Dump).expect_err("nobody is ready");

        assert!(
            matches!(
                error,
                AppError::PlatformLocator(LocatorError::PlatformReleaseMissing(_))
            ),
            "{error}"
        );
        assert!(
            !receipt.skipped.is_empty(),
            "the receipt still names the skipped"
        );
        let error = UseCaseError::from(error);
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::PlatformReleaseMissing)
        );
        let refusal = error
            .platform_release()
            .expect("the refusal carries its inventory");
        assert_eq!(refusal.source_format.release.as_deref(), Some("8.5.1"));
        assert_eq!(refusal.installations.len(), 1);
    }
}
