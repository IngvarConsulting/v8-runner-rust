//! `platforms`: установленные платформы и выбор раннера для проекта.
//!
//! Выбор считает тот же поиск, что и рабочие команды (`PlatformUtilities`), поэтому ответ не
//! расходится с тем, что команда запустит.

use std::path::PathBuf;
use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::launch::{PlatformResolution, PlatformResolutionSource};
use crate::domain::platforms::{
    PlatformInstallation, PlatformRequirement, PlatformsResult, SelectedPlatform, SelectedPlatforms,
};
use crate::platform::locator::{
    LocatorError, ResolutionSource, UtilityLocation, UtilityType, UtilityVersion,
};
use crate::platform::utilities::{sources_format, PlatformUtilities};

pub(crate) enum PlatformsScope<'a> {
    Project(&'a AppConfig),
    Inventory(Option<PathBuf>),
}

/// Отвечает, что установлено и что раннер выбрал бы, и предупреждения конверта: расхождение
/// формата исходников с выбранной платформой. Платформу не запускает.
pub(crate) fn execute(scope: PlatformsScope<'_>) -> (PlatformsResult, Vec<String>) {
    let started = Instant::now();
    let (mut utilities, requirement, source_format, warn_missing) = match scope {
        PlatformsScope::Project(config) => (
            // Расхождение формата ответ называет в `warnings`; журнал его не повторяет.
            PlatformUtilities::from_config(config).without_mismatch_log(),
            PlatformRequirement {
                version: config
                    .tools
                    .platform
                    .version
                    .as_ref()
                    .map(ToString::to_string),
                path: config.tools.platform.path.clone(),
                strict: config.tools.platform.strict,
            },
            sources_format(config).map(|format| format.info()),
            false,
        ),
        PlatformsScope::Inventory(path) => {
            let path = path.map(crate::platform::locator::inventory_search_path);
            let utilities = PlatformUtilities::for_inventory(path.clone());
            let warn_missing = path.is_some();
            (
                utilities,
                PlatformRequirement {
                    version: None,
                    path,
                    strict: false,
                },
                None,
                warn_missing,
            )
        }
    };
    let installations = utilities
        .installations()
        .iter()
        .map(PlatformInstallation::from)
        .collect();
    let mut warnings = Vec::new();
    let mut select = |utility| match utilities.locate(utility) {
        Ok(location) => {
            if let Some(mismatch) = utilities.format_mismatch(&location) {
                if !warnings.contains(&mismatch) {
                    warnings.push(mismatch);
                }
            }
            Some(selected(&location))
        }
        // При обычном поиске отсутствие утилиты показано самим значением `null`.
        Err(LocatorError::NotFound { .. }) if !warn_missing => None,
        // Для явного CLI-пути сообщаем также об отсутствии утилиты. Прочие причины
        // отказа показываем всегда. Отказ выпуска при `sources` один на все утилиты.
        Err(error) => {
            let warning = error.to_string();
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
            None
        }
    };
    let selected = SelectedPlatforms {
        designer: select(UtilityType::V8),
        thin_client: select(UtilityType::V8C),
        ibcmd: select(UtilityType::Ibcmd),
    };
    let result = PlatformsResult {
        installations,
        requirement,
        source_format,
        selected,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    (result, warnings)
}

fn selected(location: &UtilityLocation) -> SelectedPlatform {
    SelectedPlatform {
        resolution: platform_resolution(location),
        chosen_by: location.chosen_by,
    }
}

/// Найденная утилита в форме ответа: путь, версия, откуда взята и каталог установки.
pub(crate) fn platform_resolution(location: &UtilityLocation) -> PlatformResolution {
    PlatformResolution {
        path: location.path.clone(),
        version: location.version.as_ref().map(utility_version_string),
        source: match location.source {
            ResolutionSource::Explicit => PlatformResolutionSource::Explicit,
            ResolutionSource::DefaultRoot => PlatformResolutionSource::DefaultRoot,
            ResolutionSource::Path => PlatformResolutionSource::Path,
        },
        installation_root: location.installation_root.clone(),
    }
}

fn utility_version_string(version: &UtilityVersion) -> String {
    match version {
        UtilityVersion::Platform(version) => version.to_string(),
        UtilityVersion::Edt(version) => version
            .parts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("."),
    }
}
