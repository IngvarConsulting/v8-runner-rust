use crate::config::model::{AppConfig, EdtCliConfig, SourceFormat, SourceSetPurpose};
use crate::domain::platforms::PlatformChoice;
use crate::platform::dump_format::{
    read_sources_format, releases_writing, FormatVersion, RecordedFormat,
};
use crate::platform::locator::{
    EdtVersion, Locator, LocatorOptions, PlatformResolutionPolicy, PlatformVersionRequirement,
    UtilityLocation, UtilityType, UtilityVersion,
};
use crate::platform::process::{ProcessExecutor, ProcessRunner};
use tracing::{debug, warn};

/// Facade over utility discovery and standard executor selection.
pub struct PlatformUtilities {
    locator: Locator,
    standard_runner: ProcessExecutor,
    /// Формат исходников, когда версию платформы не задали ни маской, ни путём: по нему
    /// выбирается выпуск, а с ним расходится выбор новейшей.
    sources_format: Option<SourcesFormat>,
    /// Расхождение уже записано в журнал: поиск зовут на каждую утилиту.
    mismatch_logged: bool,
}

impl PlatformUtilities {
    /// Build platform utilities facade from application configuration.
    pub fn from_config(config: &AppConfig) -> Self {
        let edt_hint = config.tools.edt_cli.path.clone().filter(|path| {
            EdtCliConfig::names_location(path)
                || path.exists()
                || config.tools.edt_cli.version.is_none()
        });
        let edt_version = config
            .tools
            .edt_cli
            .version
            .as_deref()
            .and_then(EdtVersion::parse_lenient)
            .or_else(|| {
                config
                    .tools
                    .edt_cli
                    .path
                    .as_deref()
                    .filter(|path| !EdtCliConfig::names_location(path))
                    .and_then(|path| path.to_str())
                    .and_then(EdtVersion::parse_lenient)
            });
        let sources_format = (config.tools.platform.version.is_none()
            && config.tools.platform.path.is_none())
        .then(|| sources_format(config))
        .flatten();
        Self {
            locator: Locator::new(LocatorOptions {
                platform_hint: config.tools.platform.path.clone(),
                platform_version: config
                    .tools
                    .platform
                    .version
                    .as_deref()
                    .and_then(PlatformVersionRequirement::parse),
                platform_policy: if config.tools.platform.strict {
                    PlatformResolutionPolicy::Strict
                } else {
                    PlatformResolutionPolicy::Lenient
                },
                edt_hint,
                edt_version,
                format_releases: sources_format
                    .as_ref()
                    .map(SourcesFormat::releases)
                    .unwrap_or_default(),
            }),
            standard_runner: ProcessExecutor,
            sources_format,
            mismatch_logged: false,
        }
    }

    /// Resolve an executable for the requested utility.
    pub fn locate(
        &mut self,
        utility: UtilityType,
    ) -> Result<UtilityLocation, crate::platform::locator::LocatorError> {
        debug!(utility = ?utility, "locating platform utility");
        let location = self.locator.locate(utility)?;
        debug!(utility = ?utility, path = %location.path.display(), "platform utility resolved");
        if !self.mismatch_logged {
            if let Some(mismatch) = self.format_mismatch(&location) {
                warn!("{mismatch}");
                self.mismatch_logged = true;
            }
        }
        Ok(location)
    }

    /// Расхождение формата исходников и выбранной платформы: версию не задали, а выпуска,
    /// который пишет формат исходников, нет или формат раннеру не известен, и взята
    /// новейшая. `None`, если версию выбрали маска или формат.
    pub fn format_mismatch(&self, location: &UtilityLocation) -> Option<String> {
        if location.chosen_by != PlatformChoice::Newest || !location.utility.is_platform() {
            return None;
        }
        let format = self.sources_format.as_ref()?;
        let newest = match &location.version {
            Some(UtilityVersion::Platform(version)) => version.to_string(),
            _ => location.path.display().to_string(),
        };
        Some(match (format.version, format.releases().first()) {
            (None, _) => format!(
                "the sources format in '{}' is not recognized; the newest platform {newest} is used",
                format.file.display()
            ),
            (Some(version), None) => format!(
                "the sources format {version} is not known to the runner; the newest platform {newest} is used and may rewrite the sources in its own format; set tools.platform.version"
            ),
            (Some(version), Some(release)) => format!(
                "the sources format {version} is written by platform {release}, which is not installed; the newest platform {newest} is used and may rewrite the sources in its own format; set tools.platform.version"
            ),
        })
    }

    /// Без записи расхождения формата в журнал: его называет сам ответ.
    pub fn without_mismatch_log(self) -> Self {
        Self {
            mismatch_logged: true,
            ..self
        }
    }

    /// Установки платформы там, где их ищет поиск.
    pub fn installations(&self) -> Vec<crate::platform::locator::InstalledPlatform> {
        self.locator.installations()
    }

    /// Return the standard runner path for the requested utility.
    ///
    /// This stage always returns the standard non-interactive executor. Future EDT work may add a
    /// different path for `UtilityType::EdtCli` without changing call sites that go through this
    /// facade.
    pub fn runner_for(&self, _utility: UtilityType) -> &dyn ProcessRunner {
        &self.standard_runner
    }

    #[cfg(test)]
    fn with_locator(locator: Locator) -> Self {
        Self {
            locator,
            standard_runner: ProcessExecutor,
            sources_format: None,
            mismatch_logged: false,
        }
    }
}

/// Формат исходников основного набора формата Конфигуратора — атрибут `version` корня его
/// `Configuration.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesFormat {
    /// Прочитанный файл.
    pub file: std::path::PathBuf,
    /// Версия формата; `None`, если файл есть, а версии в его корне раннер не нашёл.
    pub version: Option<FormatVersion>,
}

impl SourcesFormat {
    /// Выпуски платформы, которые пишут этот формат, по таблице замеров, новейшие первыми.
    pub fn releases(&self) -> Vec<PlatformVersionRequirement> {
        self.version.map(releases_writing).unwrap_or_default()
    }
}

/// Формат исходников проекта; `None`, если формат проекта не Конфигуратора, основного набора
/// или его `Configuration.xml` нет или файл не читается.
pub fn sources_format(config: &AppConfig) -> Option<SourcesFormat> {
    if config.format != SourceFormat::Designer {
        return None;
    }
    let main = config
        .source_sets
        .iter()
        .find(|set| set.purpose == SourceSetPurpose::Configuration)?;
    let file = main.root_in(&config.base_path).join("Configuration.xml");
    let version = match read_sources_format(&file) {
        Ok(RecordedFormat::Version(version)) => Some(version),
        Ok(RecordedFormat::Unrecognized) => None,
        Ok(RecordedFormat::Missing) => return None,
        Err(error) => {
            debug!(file = %file.display(), %error, "sources format was not read");
            return None;
        }
    };
    Some(SourcesFormat { file, version })
}

#[cfg(test)]
mod tests {
    use super::PlatformUtilities;
    use crate::config::model::{
        AppConfig, InfobaseConfig, McpConfig, PlatformToolConfig, SourceFormat, TestsConfig,
        ToolsConfig,
    };
    use crate::platform::locator::{
        EdtVersion, Locator, LocatorError, PlatformVersion, PlatformVersionRequirement, UtilityType,
    };
    use crate::support::path::normalize_windows_verbatim_path;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    fn touch_executable(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        make_executable(path);
    }

    fn sample_config(platform_path: Option<PathBuf>, platform_version: Option<&str>) -> AppConfig {
        AppConfig {
            base_path: PathBuf::from("/tmp/project"),
            work_path: PathBuf::from("/tmp/project/.work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: Vec::new(),
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: platform_path,
                    strict: false,
                    version: platform_version.map(str::to_owned),
                },
                ..ToolsConfig::default()
            },
            mcp: McpConfig::default(),
            tests: TestsConfig::default(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn locate_edt_cli_uses_configured_binary_path() {
        let dir = tempdir().expect("tempdir");
        let binary = dir.path().join("1cedtcli");
        fs::write(&binary, "#!/bin/sh\nexit 0\n").expect("write");
        make_executable(&binary);
        let locator = Locator::with_roots(None, None, Some(binary.clone()), None, vec![], vec![]);
        let mut utilities = PlatformUtilities::with_locator(locator);

        let location = utilities.locate(UtilityType::EdtCli).expect("locate edt");

        assert_eq!(
            location.path,
            binary.canonicalize().expect("canonical binary")
        );
    }

    #[cfg(unix)]
    #[test]
    fn locate_edt_cli_returns_not_found_when_unconfigured() {
        let locator = Locator::with_roots(None, None, None, None, vec![], vec![]);
        let mut utilities = PlatformUtilities::with_locator(locator);

        let error = utilities
            .locate(UtilityType::EdtCli)
            .expect_err("expected not found");

        assert!(matches!(
            error,
            LocatorError::NotFound {
                utility: UtilityType::EdtCli,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn locate_edt_cli_uses_version_filtered_autodiscovery() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("components");
        let wanted = root
            .join("1c-edt-2025.2.3+30-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        let older = root
            .join("1c-edt-2025.1.9+10-x86_64")
            .join("1cedt")
            .join("1cedtcli");
        fs::create_dir_all(wanted.parent().expect("wanted parent")).expect("wanted dirs");
        fs::create_dir_all(older.parent().expect("older parent")).expect("older dirs");
        fs::write(&wanted, "#!/bin/sh\nexit 0\n").expect("wanted");
        fs::write(&older, "#!/bin/sh\nexit 0\n").expect("older");
        make_executable(&wanted);
        make_executable(&older);

        let locator = Locator::with_roots(
            None,
            None,
            None,
            Some(EdtVersion::parse_lenient("1c-edt-2025.2.3").expect("version")),
            vec![],
            vec![root],
        );
        let mut utilities = PlatformUtilities::with_locator(locator);

        let location = utilities.locate(UtilityType::EdtCli).expect("locate edt");

        assert_eq!(
            location.path,
            wanted.canonicalize().expect("canonical binary")
        );
    }

    #[cfg(unix)]
    #[test]
    fn from_config_locates_all_platform_utilities_via_shared_platform_contract() {
        for utility in [UtilityType::V8, UtilityType::V8C, UtilityType::Ibcmd] {
            let dir = tempdir().expect("tempdir");
            let root = dir
                .path()
                .join(format!("platform-{}", utility.executable_name()));
            let wanted = root
                .join("8.3.27.1789")
                .join("bin")
                .join(utility.executable_name());
            let older = root
                .join("8.3.20.9999")
                .join("bin")
                .join(utility.executable_name());
            touch_executable(&wanted);
            touch_executable(&older);

            let config = sample_config(Some(root), Some("8.3"));
            let mut utilities = PlatformUtilities::from_config(&config);

            let location = utilities.locate(utility).expect("locate platform utility");

            assert_eq!(
                location.path,
                wanted.canonicalize().expect("canonical binary")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn from_config_ignores_platform_version_for_lenient_path_hint() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let binary = root
            .join("8.4.1.1")
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&binary);
        let config = sample_config(Some(root), Some("8.3"));
        let mut utilities = PlatformUtilities::from_config(&config);

        let location = utilities
            .locate(UtilityType::V8)
            .expect("lenient path ignores configured version");

        assert_eq!(
            location.path,
            binary.canonicalize().expect("canonical binary")
        );
    }

    #[test]
    fn from_config_maps_strict_platform_flag_to_fail_closed_policy() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("platform");
        let binary = root
            .join("8.3.25.9999")
            .join("bin")
            .join(UtilityType::V8.executable_name());
        touch_executable(&binary);
        let mut config = sample_config(Some(root), Some("8.3.25.1234"));
        config.tools.platform.strict = true;
        let mut utilities = PlatformUtilities::from_config(&config);

        assert_eq!(
            utilities.locate(UtilityType::V8).expect_err("mismatch"),
            LocatorError::VersionMismatch {
                utility: UtilityType::V8,
                path: normalize_windows_verbatim_path(
                    &binary.canonicalize().expect("canonical binary")
                ),
                required: PlatformVersionRequirement::parse("8.3.25.1234").expect("requirement"),
                found: PlatformVersion::parse_strict("8.3.25.9999").expect("version"),
            }
        );
    }
}
