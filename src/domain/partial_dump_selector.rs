use crate::support::error::AppError;

pub const PARTIAL_OBJECT_BLANK_ERROR: &str = "partial dump objects must not be blank";
pub const PARTIAL_OBJECT_CONTROL_ERROR: &str =
    "partial dump objects must not contain control characters";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataRootType {
    AccountingRegister,
    AccumulationRegister,
    Bot,
    BusinessProcess,
    CalculationRegister,
    Catalog,
    ChartOfAccounts,
    ChartOfCalculationTypes,
    ChartOfCharacteristicTypes,
    CommandGroup,
    CommonAttribute,
    CommonCommand,
    CommonForm,
    CommonModule,
    CommonPicture,
    CommonTemplate,
    Configuration,
    Constant,
    DataProcessor,
    DefinedType,
    Document,
    DocumentJournal,
    DocumentNumerator,
    Enum,
    EventSubscription,
    ExchangePlan,
    ExternalDataSource,
    FilterCriterion,
    FunctionalOption,
    FunctionalOptionsParameter,
    HTTPService,
    InformationRegister,
    IntegrationService,
    Language,
    Report,
    Role,
    ScheduledJob,
    Sequence,
    SessionParameter,
    SettingsStorage,
    Style,
    StyleItem,
    Subsystem,
    Task,
    WebService,
    WebSocketClient,
    WSReference,
    XDTOPackage,
}

impl MetadataRootType {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "AccountingRegister" => Ok(Self::AccountingRegister),
            "AccumulationRegister" => Ok(Self::AccumulationRegister),
            "Bot" => Ok(Self::Bot),
            "BusinessProcess" => Ok(Self::BusinessProcess),
            "CalculationRegister" => Ok(Self::CalculationRegister),
            "Catalog" => Ok(Self::Catalog),
            "ChartOfAccounts" => Ok(Self::ChartOfAccounts),
            "ChartOfCalculationTypes" => Ok(Self::ChartOfCalculationTypes),
            "ChartOfCharacteristicTypes" => Ok(Self::ChartOfCharacteristicTypes),
            "CommandGroup" => Ok(Self::CommandGroup),
            "CommonAttribute" => Ok(Self::CommonAttribute),
            "CommonCommand" => Ok(Self::CommonCommand),
            "CommonForm" => Ok(Self::CommonForm),
            "CommonModule" => Ok(Self::CommonModule),
            "CommonPicture" => Ok(Self::CommonPicture),
            "CommonTemplate" => Ok(Self::CommonTemplate),
            "Configuration" => Ok(Self::Configuration),
            "Constant" => Ok(Self::Constant),
            "DataProcessor" => Ok(Self::DataProcessor),
            "DefinedType" => Ok(Self::DefinedType),
            "Document" => Ok(Self::Document),
            "DocumentJournal" => Ok(Self::DocumentJournal),
            "DocumentNumerator" => Ok(Self::DocumentNumerator),
            "Enum" => Ok(Self::Enum),
            "EventSubscription" => Ok(Self::EventSubscription),
            "ExchangePlan" => Ok(Self::ExchangePlan),
            "ExternalDataSource" => Ok(Self::ExternalDataSource),
            "FilterCriterion" => Ok(Self::FilterCriterion),
            "FunctionalOption" => Ok(Self::FunctionalOption),
            "FunctionalOptionsParameter" => Ok(Self::FunctionalOptionsParameter),
            "HTTPService" => Ok(Self::HTTPService),
            "InformationRegister" => Ok(Self::InformationRegister),
            "IntegrationService" => Ok(Self::IntegrationService),
            "Language" => Ok(Self::Language),
            "Report" => Ok(Self::Report),
            "Role" => Ok(Self::Role),
            "ScheduledJob" => Ok(Self::ScheduledJob),
            "Sequence" => Ok(Self::Sequence),
            "SessionParameter" => Ok(Self::SessionParameter),
            "SettingsStorage" => Ok(Self::SettingsStorage),
            "Style" => Ok(Self::Style),
            "StyleItem" => Ok(Self::StyleItem),
            "Subsystem" => Ok(Self::Subsystem),
            "Task" => Ok(Self::Task),
            "WebService" => Ok(Self::WebService),
            "WebSocketClient" => Ok(Self::WebSocketClient),
            "WSReference" => Ok(Self::WSReference),
            "XDTOPackage" => Ok(Self::XDTOPackage),
            _ => Err(AppError::Validation(format!(
                "partial dump object has unknown metadata root type: {value}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AccountingRegister => "AccountingRegister",
            Self::AccumulationRegister => "AccumulationRegister",
            Self::Bot => "Bot",
            Self::BusinessProcess => "BusinessProcess",
            Self::CalculationRegister => "CalculationRegister",
            Self::Catalog => "Catalog",
            Self::ChartOfAccounts => "ChartOfAccounts",
            Self::ChartOfCalculationTypes => "ChartOfCalculationTypes",
            Self::ChartOfCharacteristicTypes => "ChartOfCharacteristicTypes",
            Self::CommandGroup => "CommandGroup",
            Self::CommonAttribute => "CommonAttribute",
            Self::CommonCommand => "CommonCommand",
            Self::CommonForm => "CommonForm",
            Self::CommonModule => "CommonModule",
            Self::CommonPicture => "CommonPicture",
            Self::CommonTemplate => "CommonTemplate",
            Self::Configuration => "Configuration",
            Self::Constant => "Constant",
            Self::DataProcessor => "DataProcessor",
            Self::DefinedType => "DefinedType",
            Self::Document => "Document",
            Self::DocumentJournal => "DocumentJournal",
            Self::DocumentNumerator => "DocumentNumerator",
            Self::Enum => "Enum",
            Self::EventSubscription => "EventSubscription",
            Self::ExchangePlan => "ExchangePlan",
            Self::ExternalDataSource => "ExternalDataSource",
            Self::FilterCriterion => "FilterCriterion",
            Self::FunctionalOption => "FunctionalOption",
            Self::FunctionalOptionsParameter => "FunctionalOptionsParameter",
            Self::HTTPService => "HTTPService",
            Self::InformationRegister => "InformationRegister",
            Self::IntegrationService => "IntegrationService",
            Self::Language => "Language",
            Self::Report => "Report",
            Self::Role => "Role",
            Self::ScheduledJob => "ScheduledJob",
            Self::Sequence => "Sequence",
            Self::SessionParameter => "SessionParameter",
            Self::SettingsStorage => "SettingsStorage",
            Self::Style => "Style",
            Self::StyleItem => "StyleItem",
            Self::Subsystem => "Subsystem",
            Self::Task => "Task",
            Self::WebService => "WebService",
            Self::WebSocketClient => "WebSocketClient",
            Self::WSReference => "WSReference",
            Self::XDTOPackage => "XDTOPackage",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialDumpSelector {
    requested: String,
    root_type: MetadataRootType,
    name: String,
}

impl PartialDumpSelector {
    pub fn parse(requested: &str) -> Result<Self, AppError> {
        if requested.chars().any(char::is_control) {
            return Err(AppError::Validation(
                PARTIAL_OBJECT_CONTROL_ERROR.to_owned(),
            ));
        }

        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation(PARTIAL_OBJECT_BLANK_ERROR.to_owned()));
        }
        let separator_count = trimmed
            .chars()
            .filter(|character| matches!(character, ':' | '.'))
            .count();
        if separator_count != 1 {
            return Err(AppError::Validation(
                "partial dump object must use exactly one ':' or '.' separator".to_owned(),
            ));
        }

        let mut parts = trimmed.split([':', '.']);
        let Some(root_type) = parts.next() else {
            return Err(AppError::Validation(
                "partial dump object must use exactly one ':' or '.' separator".to_owned(),
            ));
        };
        let Some(name) = parts.next() else {
            return Err(AppError::Validation(
                "partial dump object must use exactly one ':' or '.' separator".to_owned(),
            ));
        };
        let root_type = MetadataRootType::parse(root_type.trim())?;
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Validation(PARTIAL_OBJECT_BLANK_ERROR.to_owned()));
        }

        Ok(Self {
            requested: requested.to_owned(),
            root_type,
            name: name.to_owned(),
        })
    }

    pub fn requested(&self) -> &str {
        &self.requested
    }

    pub fn normalized(&self) -> String {
        format!("{}.{}", self.root_type.as_str(), self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::PartialDumpSelector;

    #[test]
    fn partial_dump_selector_normalizes_colon_separator() {
        let selector = PartialDumpSelector::parse("Catalog:Items").expect("selector");

        assert_eq!(selector.requested(), "Catalog:Items");
        assert_eq!(selector.normalized(), "Catalog.Items");
    }

    #[test]
    fn partial_dump_selector_preserves_outer_whitespace_in_requested_value() {
        let selector = PartialDumpSelector::parse("  Catalog:Items  ").expect("selector");

        assert_eq!(selector.requested(), "  Catalog:Items  ");
        assert_eq!(selector.normalized(), "Catalog.Items");
    }

    #[test]
    fn partial_dump_selector_accepts_dotted_compatibility_form() {
        let selector = PartialDumpSelector::parse("Catalog.Items").expect("selector");

        assert_eq!(selector.requested(), "Catalog.Items");
        assert_eq!(selector.normalized(), "Catalog.Items");
    }

    #[test]
    fn partial_dump_selector_accepts_fixture_metadata_root_types() {
        for (requested, normalized) in [
            (
                "ChartOfCharacteristicTypes:ПланВидовХарактеристик1",
                "ChartOfCharacteristicTypes.ПланВидовХарактеристик1",
            ),
            ("CommandGroup:ГруппаКоманд1", "CommandGroup.ГруппаКоманд1"),
            (
                "CommonPicture:ОбщаяКартинка1",
                "CommonPicture.ОбщаяКартинка1",
            ),
            ("CommonTemplate:Макет", "CommonTemplate.Макет"),
            ("Configuration:Конфигурация", "Configuration.Конфигурация"),
            (
                "DocumentNumerator:НумераторДокументов1",
                "DocumentNumerator.НумераторДокументов1",
            ),
            (
                "ExternalDataSource:ВнешнийИсточникДанных1",
                "ExternalDataSource.ВнешнийИсточникДанных1",
            ),
            (
                "SettingsStorage:ХранилищеНастроек1",
                "SettingsStorage.ХранилищеНастроек1",
            ),
            ("StyleItem:ЭлементСтиля1", "StyleItem.ЭлементСтиля1"),
            (
                "WebSocketClient:WebSocketКлиент1",
                "WebSocketClient.WebSocketКлиент1",
            ),
        ] {
            let selector = PartialDumpSelector::parse(requested).expect("fixture selector");

            assert_eq!(selector.requested(), requested);
            assert_eq!(selector.normalized(), normalized);
        }
    }

    #[test]
    fn partial_dump_selector_rejects_invalid_forms() {
        for requested in [
            "Unknown:Items",
            "Catalog:",
            "Catalog:Items:Extra",
            "Catalog:Item\nInjected",
        ] {
            assert!(
                PartialDumpSelector::parse(requested).is_err(),
                "expected '{requested}' to be rejected"
            );
        }
    }
}
