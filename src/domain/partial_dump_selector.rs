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
    CommonAttribute,
    CommonCommand,
    CommonForm,
    CommonModule,
    Constant,
    DataProcessor,
    DefinedType,
    Document,
    DocumentJournal,
    Enum,
    EventSubscription,
    ExchangePlan,
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
    Style,
    Subsystem,
    Task,
    WebService,
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
            "CommonAttribute" => Ok(Self::CommonAttribute),
            "CommonCommand" => Ok(Self::CommonCommand),
            "CommonForm" => Ok(Self::CommonForm),
            "CommonModule" => Ok(Self::CommonModule),
            "Constant" => Ok(Self::Constant),
            "DataProcessor" => Ok(Self::DataProcessor),
            "DefinedType" => Ok(Self::DefinedType),
            "Document" => Ok(Self::Document),
            "DocumentJournal" => Ok(Self::DocumentJournal),
            "Enum" => Ok(Self::Enum),
            "EventSubscription" => Ok(Self::EventSubscription),
            "ExchangePlan" => Ok(Self::ExchangePlan),
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
            "Style" => Ok(Self::Style),
            "Subsystem" => Ok(Self::Subsystem),
            "Task" => Ok(Self::Task),
            "WebService" => Ok(Self::WebService),
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
            Self::CommonAttribute => "CommonAttribute",
            Self::CommonCommand => "CommonCommand",
            Self::CommonForm => "CommonForm",
            Self::CommonModule => "CommonModule",
            Self::Constant => "Constant",
            Self::DataProcessor => "DataProcessor",
            Self::DefinedType => "DefinedType",
            Self::Document => "Document",
            Self::DocumentJournal => "DocumentJournal",
            Self::Enum => "Enum",
            Self::EventSubscription => "EventSubscription",
            Self::ExchangePlan => "ExchangePlan",
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
            Self::Style => "Style",
            Self::Subsystem => "Subsystem",
            Self::Task => "Task",
            Self::WebService => "WebService",
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

        let requested = requested.trim();
        if requested.is_empty() {
            return Err(AppError::Validation(PARTIAL_OBJECT_BLANK_ERROR.to_owned()));
        }
        let separator_count = requested
            .chars()
            .filter(|character| matches!(character, ':' | '.'))
            .count();
        if separator_count != 1 {
            return Err(AppError::Validation(
                "partial dump object must use exactly one ':' or '.' separator".to_owned(),
            ));
        }

        let mut parts = requested.split(|character| matches!(character, ':' | '.'));
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
    fn partial_dump_selector_accepts_dotted_compatibility_form() {
        let selector = PartialDumpSelector::parse("Catalog.Items").expect("selector");

        assert_eq!(selector.requested(), "Catalog.Items");
        assert_eq!(selector.normalized(), "Catalog.Items");
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
