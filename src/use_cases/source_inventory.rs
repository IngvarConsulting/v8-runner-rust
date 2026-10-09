use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::ContextAnalysis;
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::{AppConfig, SourceSetConfig, SourceSetPurpose};
use crate::domain::infobase_export::TransferArtifactKind;
use crate::domain::next_step::NextStep;
use crate::domain::source_set::SourceSetContext;
use crate::platform::extension_inventory::is_windows_device_name;
use crate::support::error::AppError;
use crate::use_cases::context::CommandName;
use crate::use_cases::extension_identity::{
    extension_name_key, platform_extension_name, source_extension_name,
};
use crate::use_cases::result::UseCaseError;

/// Наборы в порядке обработки: основная конфигурация, расширения, внешние обработки,
/// внешние отчёты, а внутри назначения — в порядке объявления.
///
/// Порядок по назначению решается здесь: обходы наборов берут его отсюда, а не собирают
/// свои корзины (страж — `tests/architecture_guardrails.rs::the_order_of_source_sets_is_decided_in_one_place`).
pub(crate) fn ordered_by_purpose(source_sets: &[SourceSetConfig]) -> Vec<&SourceSetConfig> {
    let rank = |purpose: SourceSetPurpose| match purpose {
        SourceSetPurpose::Configuration => 0,
        SourceSetPurpose::Extension => 1,
        SourceSetPurpose::ExternalDataProcessors => 2,
        SourceSetPurpose::ExternalReports => 3,
    };
    let mut ordered = source_sets.iter().collect::<Vec<_>>();
    // Сортировка устойчивая: внутри назначения остаётся порядок объявления.
    ordered.sort_by_key(|source_set| rank(source_set.purpose));
    ordered
}

/// Read-only runtime index for source-set orchestration.
pub(crate) struct SourceSetInventory<'a> {
    config: &'a AppConfig,
    source_sets_by_name: HashMap<&'a str, &'a SourceSetConfig>,
    designer_contexts: Vec<SourceSetContext>,
    designer_contexts_by_name: HashMap<String, SourceSetContext>,
    edt_contexts: Vec<SourceSetContext>,
    edt_contexts_by_name: HashMap<String, SourceSetContext>,
}

impl<'a> SourceSetInventory<'a> {
    pub(crate) fn new(config: &'a AppConfig) -> Self {
        let service = SourceSetsService::new(config);
        let designer_contexts = service.designer_contexts();
        let edt_contexts = service.edt_contexts();

        Self {
            config,
            source_sets_by_name: config
                .source_sets
                .iter()
                .map(|source_set| (source_set.name.as_str(), source_set))
                .collect(),
            designer_contexts_by_name: index_contexts(&designer_contexts),
            designer_contexts,
            edt_contexts_by_name: index_contexts(&edt_contexts),
            edt_contexts,
        }
    }

    pub(crate) fn source_sets(&self) -> Vec<&'a SourceSetConfig> {
        self.config.source_sets.iter().collect()
    }

    pub(crate) fn ordered_source_sets(&self) -> Vec<&'a SourceSetConfig> {
        ordered_by_purpose(&self.config.source_sets)
    }

    /// Пакеты конфигурации проекта в порядке обхода [`Self::ordered_source_sets`]: основная
    /// конфигурация (`None`), затем расширения — каждое с именем расширения в базе. Наборы
    /// внешних файлов пакета конфигурации не называют и в обход не входят.
    ///
    /// Один порядок на все команды, которые идут по пакетам без аргумента: так `pull --all`
    /// выгружает, и тем же порядком идут `make` и `download` без набора (#364).
    pub(crate) fn configuration_packages(&self) -> Vec<(&'a SourceSetConfig, Option<&'a str>)> {
        self.ordered_source_sets()
            .into_iter()
            .filter_map(|source_set| match source_set.purpose {
                SourceSetPurpose::Configuration => Some((source_set, None)),
                SourceSetPurpose::Extension => {
                    Some((source_set, Some(platform_extension_name(source_set))))
                }
                SourceSetPurpose::ExternalDataProcessors | SourceSetPurpose::ExternalReports => {
                    None
                }
            })
            .collect()
    }

    /// Каталог, в который `make` и `download` без набора кладут пакет каждого набора
    /// ([`package_in_directory`]). Путь, который называет файл — с суффиксом и не
    /// существующий каталог, или существующий файл, — отказ: без набора команда пишет не один
    /// файл, а `next` называет ту же команду с набором основной конфигурации и файлом `.cf`.
    ///
    /// `relative_to` — откуда команда считает относительный путь: `download` — от
    /// `basePath`, `make` (`None`) — от текущего каталога. Ответ — каталог, разрешённый от
    /// этой точки: его называет ответ команды и с ним сверяются цели пакетов.
    pub(crate) fn packages_directory(
        &self,
        command: CommandName,
        output: &str,
        relative_to: Option<&Path>,
    ) -> Result<PathBuf, UseCaseError> {
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation(format!(
                "{} without <SET> requires --output naming a directory",
                command.as_str()
            ))
            .into());
        }
        let directory = PathBuf::from(trimmed);
        let resolved = match relative_to {
            Some(base) => crate::support::path::resolve_from(base, &directory),
            None => std::path::absolute(&directory)
                .map(|path| crate::support::path::lexically_normal_absolute(&path))
                .map_err(|error| {
                    AppError::Runtime(format!(
                        "failed to resolve --output '{trimmed}' against the current directory: {error}"
                    ))
                })?,
        };
        if !names_a_file(&directory, &resolved) {
            return Ok(resolved);
        }
        let mut error = UseCaseError::from(AppError::Validation(format!(
            "{command} without <SET> writes a package for each source-set into the directory --output names, and '{trimmed}' names a file: name a directory, or name the source-set to write one package",
            command = command.as_str()
        )));
        let main = self
            .configuration_packages()
            .into_iter()
            .find_map(|(source_set, extension)| extension.is_none().then_some(source_set));
        if let Some(main) = main {
            let file = directory.with_extension(TransferArtifactKind::Cf.file_extension());
            error = error.with_next(
                NextStep::command(command.as_str())
                    .for_source_set(main.name.clone())
                    .with_key("--output", file.display().to_string()),
            );
        }
        Err(error)
    }

    /// Пакеты конфигурации проекта по составу базы: набор расширения есть в базе, когда в
    /// `installed` есть его расширение ([`extension_name_key`]: 1С регистр в именах не
    /// различает). Сопоставление одно на все обходы по составу базы — `pull --all` и
    /// `download` без набора.
    ///
    /// Набор, исходники которого называют другое установленное расширение, — отказ: по
    /// имени набора он взял бы не то расширение (#218).
    pub(crate) fn installed_packages(
        &self,
        installed: &[String],
    ) -> Result<InstalledPackages<'a>, AppError> {
        let installed = installed
            .iter()
            .map(|name| extension_name_key(name))
            .collect::<HashSet<_>>();
        let mut packages = InstalledPackages {
            present: Vec::new(),
            not_installed: Vec::new(),
        };
        for (source_set, extension) in self.configuration_packages() {
            let Some(extension) = extension else {
                packages.present.push((source_set, None));
                continue;
            };
            let key = extension_name_key(extension);
            let root = source_set.root_in(&self.config.base_path);
            if let Some(held) = source_extension_name(self.config.format, &root)?.filter(|held| {
                let held = extension_name_key(held);
                held != key && installed.contains(&held)
            }) {
                return Err(AppError::Validation(format!(
                    "source-set '{}' holds extension '{held}' by the Name in its sources, but the runner takes an extension set by the set's name, '{extension}' (#218): rename the set to '{held}' in the project file; no second set is declared for '{held}'",
                    source_set.name
                )));
            }
            if installed.contains(&key) {
                packages.present.push((source_set, Some(extension)));
            } else {
                packages.not_installed.push(source_set);
            }
        }
        Ok(packages)
    }

    /// Цели пакетов обхода без набора проверяются до работы. Пакет не ложится на каталог
    /// набора или `workPath`, внутрь них и вокруг них ([`paths_overlap`]): публикация
    /// каталога внешнего набора заменила бы исходники. Два пакета не получают одно имя на
    /// файловой системе без регистра, и имя пакета не бывает именем устройства Windows.
    pub(crate) fn check_package_targets(
        &self,
        command: CommandName,
        directory: &Path,
        sets: &[&SourceSetConfig],
    ) -> Result<(), AppError> {
        let command = command.as_str();
        let guarded = self
            .config
            .source_sets
            .iter()
            .map(|source_set| {
                (
                    format!("the directory of source-set '{}'", source_set.name),
                    comparable_path(&source_set.root_in(&self.config.base_path)),
                )
            })
            .chain(std::iter::once((
                "workPath".to_owned(),
                comparable_path(&self.config.work_path),
            )))
            .collect::<Vec<_>>();
        let mut names = HashMap::new();
        for source_set in sets {
            if is_windows_device_name(&source_set.name) {
                return Err(AppError::Validation(format!(
                    "{command} without <SET> names the package of source-set '{0}' after the set, and '{0}' is a Windows device name: name the set to write its package to a file of your choice",
                    source_set.name
                )));
            }
            let target = package_in_directory(directory, source_set);
            // Ключ — имя файла или каталога пакета, а не имя набора: расширение `Sales` и
            // внешний набор `Sales.cfe` оба легли бы в `Sales.cfe`.
            let file_name = target
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            if let Some(other) = names.insert(extension_name_key(&file_name), &source_set.name) {
                return Err(AppError::Validation(format!(
                    "{command} without <SET> would give source-sets '{other}' and '{}' the same package file name '{file_name}' on a case-insensitive file system: name each set to write its package",
                    source_set.name
                )));
            }
            let comparable = comparable_path(&target);
            if let Some((what, _)) = guarded
                .iter()
                .find(|(_, guarded)| paths_overlap(&comparable, guarded))
            {
                return Err(AppError::Validation(format!(
                    "{command} without <SET> would write the package of source-set '{}' to '{}', which overlaps {what}: name a directory outside the project sources and workPath",
                    source_set.name,
                    target.display()
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn source_set(&self, name: &str) -> Option<&'a SourceSetConfig> {
        self.source_sets_by_name.get(name).copied()
    }

    /// Набор, названный пользователем. Имя, которого нет среди наборов проекта, — отказ
    /// одной формулировкой для всех команд.
    pub(crate) fn named(&self, name: &str) -> Result<&'a SourceSetConfig, AppError> {
        self.source_set(name)
            .ok_or_else(|| AppError::Validation(format!("unknown source-set '{name}'")))
    }

    /// Пакет конфигурации, который называет набор: набор конфигурации — основную
    /// конфигурацию (`None`), набор расширения — расширение с именем набора. У набора
    /// внешних файлов пакета конфигурации нет: отказ называет команду `command`, которая
    /// его просила.
    pub(crate) fn configuration_package(
        &self,
        name: &str,
        command: CommandName,
    ) -> Result<(&'a SourceSetConfig, Option<&'a str>), AppError> {
        let source_set = self.named(name)?;
        match source_set.purpose {
            SourceSetPurpose::Configuration => Ok((source_set, None)),
            SourceSetPurpose::Extension => {
                Ok((source_set, Some(platform_extension_name(source_set))))
            }
            SourceSetPurpose::ExternalDataProcessors | SourceSetPurpose::ExternalReports => {
                Err(AppError::Validation(format!(
                    "source-set '{name}' holds external files; {} takes the main configuration or an extension",
                    command.as_str()
                )))
            }
        }
    }

    pub(crate) fn source_sets_with_purpose(
        &self,
        purpose: SourceSetPurpose,
    ) -> Vec<&'a SourceSetConfig> {
        self.config
            .source_sets
            .iter()
            .filter(|source_set| source_set.purpose == purpose)
            .collect()
    }

    /// Набор основной конфигурации — первый набор конфигурации в порядке объявления: на
    /// него расширение собирается у `make`, из него собирается созданная файловая база.
    pub(crate) fn main_configuration(&self) -> Option<&'a SourceSetConfig> {
        self.source_sets_with_purpose(SourceSetPurpose::Configuration)
            .into_iter()
            .next()
    }

    pub(crate) fn source_path(&self, source_set: &SourceSetConfig) -> PathBuf {
        source_set.root_in(&self.config.base_path)
    }

    pub(crate) fn designer_contexts(&self) -> &[SourceSetContext] {
        &self.designer_contexts
    }

    pub(crate) fn designer_context(&self, source_set_name: &str) -> Option<&SourceSetContext> {
        self.designer_contexts_by_name.get(source_set_name)
    }

    pub(crate) fn edt_contexts(&self) -> &[SourceSetContext] {
        &self.edt_contexts
    }

    pub(crate) fn edt_context(&self, source_set_name: &str) -> Option<&SourceSetContext> {
        self.edt_contexts_by_name.get(source_set_name)
    }

    pub(crate) fn has_edt_contexts(&self) -> bool {
        !self.edt_contexts.is_empty()
    }

    pub(crate) fn analyze_contexts(
        &self,
        contexts: &[SourceSetContext],
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Vec<ContextAnalysis> {
        SourceSetsService::new(self.config).analyze_contexts(contexts, interrupted)
    }
}

/// Называет ли путь `--output` файл, а не каталог: существующий — по тому, что лежит на
/// диске, несуществующий — по суффиксу в написанном `written`. Одно правило для `make`,
/// `download` и `convert --to package`.
pub(crate) fn names_a_file(written: &Path, resolved: &Path) -> bool {
    if resolved.exists() {
        !resolved.is_dir()
    } else {
        written.extension().is_some()
    }
}

/// Куда `make` и `download` без набора кладут пакет набора: `<каталог>/<SET>.cf` у набора
/// конфигурации, `<каталог>/<SET>.cfe` у набора расширения и каталог `<каталог>/<SET>` у
/// набора внешних файлов. Имя файла — имя набора: оно в проекте единственно.
pub(crate) fn package_in_directory(directory: &Path, source_set: &SourceSetConfig) -> PathBuf {
    let path = directory.join(&source_set.name);
    let suffix = match source_set.purpose {
        SourceSetPurpose::Configuration => TransferArtifactKind::Cf.file_extension(),
        SourceSetPurpose::Extension => TransferArtifactKind::Cfe.file_extension(),
        SourceSetPurpose::ExternalDataProcessors | SourceSetPurpose::ExternalReports => {
            return path
        }
    };
    let mut file = path.into_os_string();
    file.push(".");
    file.push(suffix);
    PathBuf::from(file)
}

/// Пакеты конфигурации проекта по составу базы ([`SourceSetInventory::installed_packages`]).
#[derive(Debug)]
pub(crate) struct InstalledPackages<'a> {
    /// Пакеты в порядке обхода, которые в базе есть: основная конфигурация (`None`) и
    /// расширения с именем расширения.
    pub(crate) present: Vec<(&'a SourceSetConfig, Option<&'a str>)>,
    /// Наборы расширений проекта, которых в базе нет.
    pub(crate) not_installed: Vec<&'a SourceSetConfig>,
}

/// Путь, сравнимый с другими до своего появления: канонический путь ближайшего
/// существующего предка с хвостом.
pub(crate) fn comparable_path(path: &Path) -> PathBuf {
    crate::support::path::nearest_existing_canonical_path(path).unwrap_or_else(|_| path.to_owned())
}

/// Пути пересекаются, когда совпадают или один лежит внутри другого: замена одного задела бы
/// другой. Сравниваются пути [`comparable_path`].
pub(crate) fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn index_contexts(contexts: &[SourceSetContext]) -> HashMap<String, SourceSetContext> {
    contexts
        .iter()
        .cloned()
        .map(|context| (context.name().to_owned(), context))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{CommandName, SourceSetInventory};
    use crate::config::model::{
        AppConfig, InfobaseConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
    };

    fn config(format: SourceFormat) -> AppConfig {
        let root = std::env::current_dir()
            .expect("current dir")
            .join("target/source-inventory-tests");
        AppConfig {
            base_path: root.join("base"),
            work_path: root.join("work"),
            format,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "ext".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: "extensions/ext".into(),
                },
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: "configuration".into(),
                },
                SourceSetConfig {
                    name: "processors".to_owned(),
                    purpose: SourceSetPurpose::ExternalDataProcessors,
                    path: "external/processors".into(),
                },
                SourceSetConfig {
                    name: "reports".to_owned(),
                    purpose: SourceSetPurpose::ExternalReports,
                    path: "external/reports".into(),
                },
            ],
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn ordered_source_sets_group_configuration_extensions_and_external_sets() {
        let config = config(SourceFormat::Designer);
        let inventory = SourceSetInventory::new(&config);

        let names = inventory
            .ordered_source_sets()
            .into_iter()
            .map(|source_set| source_set.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["main", "ext", "processors", "reports"]);
    }

    /// Пакеты конфигурации обходятся в одном порядке: основная конфигурация, затем
    /// расширения в порядке объявления; наборы внешних файлов в обход не входят.
    #[test]
    fn configuration_packages_are_walked_in_one_order() {
        let mut config = config(SourceFormat::Designer);
        config.source_sets.push(SourceSetConfig {
            name: "later".to_owned(),
            purpose: SourceSetPurpose::Extension,
            path: "extensions/later".into(),
        });
        let inventory = SourceSetInventory::new(&config);

        let packages = inventory
            .configuration_packages()
            .into_iter()
            .map(|(source_set, extension)| (source_set.name.as_str(), extension))
            .collect::<Vec<_>>();

        assert_eq!(
            packages,
            vec![
                ("main", None),
                ("ext", Some("ext")),
                ("later", Some("later"))
            ]
        );
    }

    /// Набор называет пакет конфигурации: основную конфигурацию или расширение с именем
    /// набора; набор внешних файлов и чужое имя — отказ.
    #[test]
    fn a_source_set_names_its_configuration_package() {
        let config = config(SourceFormat::Designer);
        let inventory = SourceSetInventory::new(&config);

        let package = |name| {
            inventory
                .configuration_package(name, CommandName::InfobaseConfigurationExport)
                .map(|(source_set, extension)| (source_set.name.as_str(), extension))
                .map_err(|error| error.to_string())
        };

        assert_eq!(package("main"), Ok(("main", None)));
        assert_eq!(package("ext"), Ok(("ext", Some("ext"))));
        let external = package("reports").expect_err("external files");
        assert!(external.contains("download takes"), "{external}");
        let unknown = package("missing").expect_err("unknown");
        assert!(
            unknown.contains("unknown source-set 'missing'"),
            "{unknown}"
        );
    }

    #[test]
    fn indexes_designer_and_edt_contexts_by_source_set_identity() {
        let config = config(SourceFormat::Edt);
        let inventory = SourceSetInventory::new(&config);

        let main = inventory.source_set("main").expect("main source-set");
        assert_eq!(
            inventory.source_path(main),
            config.base_path.join("configuration")
        );
        // Снимок Конфигуратора лежит под памятью базы, названной строкой соединения.
        let snapshot = inventory.designer_context("main").expect("designer").path();
        assert!(snapshot.starts_with(config.work_path.join("infobases")));
        assert!(snapshot.ends_with("designer/main"));
        assert_eq!(
            inventory.edt_context("main").expect("edt").path(),
            config.base_path.join("configuration").as_path()
        );
    }

    /// Пакет набора называется именем набора: `.cf` у конфигурации, `.cfe` у расширения,
    /// каталог у внешних файлов.
    #[test]
    fn a_package_is_named_after_its_set() {
        let config = config(SourceFormat::Designer);
        let inventory = SourceSetInventory::new(&config);
        let dir = std::path::Path::new("dist");
        let named =
            |name: &str| super::package_in_directory(dir, inventory.source_set(name).expect("set"));
        assert_eq!(named("main"), dir.join("main.cf"));
        assert_eq!(named("ext"), dir.join("ext.cfe"));
        assert_eq!(named("processors"), dir.join("processors"));
        assert_eq!(named("reports"), dir.join("reports"));
    }

    /// Файл без набора — отказ с шагом к набору основной конфигурации и файлу `.cf`; каталог
    /// принимается.
    #[test]
    fn a_file_output_without_a_set_names_the_main_set() {
        let config = config(SourceFormat::Designer);
        let inventory = SourceSetInventory::new(&config);
        let dir = tempfile::tempdir().expect("tempdir");
        let existing_file = dir.path().join("present");
        std::fs::write(&existing_file, "x").expect("file");

        assert_eq!(
            inventory.packages_directory(CommandName::Artifacts, "dist", None),
            Ok(std::env::current_dir().expect("cwd").join("dist"))
        );
        let existing_dir = dir.path().join("dist.v2");
        std::fs::create_dir_all(&existing_dir).expect("dir");
        let existing_dir = existing_dir.display().to_string();
        assert!(inventory
            .packages_directory(CommandName::Artifacts, &existing_dir, None)
            .is_ok());

        for (output, file) in [
            ("dist/release.cfe".to_owned(), "dist/release.cf".to_owned()),
            (
                existing_file.display().to_string(),
                format!("{}.cf", existing_file.display()),
            ),
        ] {
            let error = inventory
                .packages_directory(CommandName::InfobaseConfigurationExport, &output, None)
                .expect_err("a file is refused");
            assert_eq!(
                error.kind(),
                crate::use_cases::result::UseCaseErrorKind::Validation
            );
            let next = error.next().expect("next step");
            assert_eq!(next.command, "download");
            assert_eq!(next.source_set.as_deref(), Some("main"));
            assert_eq!(next.keys.get("--output"), Some(&file), "{output}");
        }
    }
}
