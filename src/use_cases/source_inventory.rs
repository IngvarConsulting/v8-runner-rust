use std::collections::HashMap;
use std::path::PathBuf;

use crate::change_detection::analyzer::ContextAnalysis;
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::{AppConfig, SourceSetConfig, SourceSetPurpose};
use crate::domain::source_set::SourceSetContext;
use crate::support::error::AppError;
use crate::use_cases::context::CommandName;
use crate::use_cases::extension_identity::platform_extension_name;

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

    pub(crate) fn analyze_contexts(&self, contexts: &[SourceSetContext]) -> Vec<ContextAnalysis> {
        SourceSetsService::new(self.config).analyze_contexts(contexts)
    }
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
}
