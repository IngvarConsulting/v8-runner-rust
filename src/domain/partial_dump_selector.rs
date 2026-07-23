use crate::support::error::AppError;

pub const PARTIAL_OBJECT_BLANK_ERROR: &str = "partial dump objects must not be blank";
pub const PARTIAL_OBJECT_CONTROL_ERROR: &str =
    "partial dump objects must not contain control characters";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialDumpSelector {
    requested: String,
    root_type: String,
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
        let root_type = root_type.trim();
        let name = name.trim();
        if root_type.is_empty() || name.is_empty() {
            return Err(AppError::Validation(PARTIAL_OBJECT_BLANK_ERROR.to_owned()));
        }

        Ok(Self {
            requested: requested.to_owned(),
            root_type: root_type.to_owned(),
            name: name.to_owned(),
        })
    }

    pub fn requested(&self) -> &str {
        &self.requested
    }

    pub fn normalized(&self) -> String {
        format!("{}.{}", self.root_type, self.name)
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
    fn partial_dump_selector_accepts_arbitrary_metadata_root_types() {
        for (requested, normalized) in [
            ("FutureRoot:Items", "FutureRoot.Items"),
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
            ":Items",
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
