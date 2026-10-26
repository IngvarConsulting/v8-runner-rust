//! `platforms`: установленные платформы и выбор раннера для проекта.
//!
//! Выбор считает тот же поиск, что и рабочие команды (`PlatformUtilities`), поэтому ответ не
//! расходится с тем, что команда запустит.

use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::launch::{PlatformResolution, PlatformResolutionSource};
use crate::domain::platforms::{
    InstalledComponents, PlatformInstallation, PlatformRequirement, PlatformsResult,
    SelectedPlatform, SelectedPlatforms, SourceFormatInfo,
};
use crate::platform::locator::{
    LocatorError, ResolutionSource, UtilityLocation, UtilityType, UtilityVersion,
};
use crate::platform::utilities::{sources_format, PlatformUtilities};

/// Отвечает, что установлено и что раннер выбрал бы, и предупреждения конверта: расхождение
/// формата исходников с выбранной платформой. Платформу не запускает.
pub fn execute(config: &AppConfig) -> (PlatformsResult, Vec<String>) {
    let started = Instant::now();
    // Расхождение формата ответ называет в `warnings`; журнал его не повторяет.
    let mut utilities = PlatformUtilities::from_config(config).without_mismatch_log();
    let installations = utilities
        .installations()
        .into_iter()
        .map(|installed| PlatformInstallation {
            version: installed.version.to_string(),
            components: InstalledComponents {
                designer: installed.has(UtilityType::V8),
                thin_client: installed.has(UtilityType::V8C),
                ibcmd: installed.has(UtilityType::Ibcmd),
                web_extension: installed.has(UtilityType::Webinst),
            },
            root: installed.root,
        })
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
        // Утилиты нет ни в одной подходящей установке: это и значит `null`.
        Err(LocatorError::NotFound { .. }) => None,
        // Утилита есть, но выбрать её нельзя: причина нужна вызывающему.
        Err(error) => {
            warnings.push(error.to_string());
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
        requirement: PlatformRequirement {
            version: config.tools.platform.version.clone(),
            path: config.tools.platform.path.clone(),
            strict: config.tools.platform.strict,
        },
        source_format: sources_format(config).map(|format| SourceFormatInfo {
            version: format.version.map(|version| version.to_string()),
            release: format.releases().first().map(ToString::to_string),
            file: format.file,
        }),
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
