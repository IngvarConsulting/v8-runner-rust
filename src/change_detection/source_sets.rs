use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::{self, ContextAnalysis};
use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig};
use crate::domain::source_set::{connection_memory_key, infobase_memory_dir, SourceSetContext};

/// Builds the list of [`SourceSetContext`] instances for the given config.
///
/// - `DESIGNER` format: one context per source-set, rooted at project base path + `ss.path`.
/// - `EDT` format (Wave 2): two contexts per source-set — the original EDT path
///   and a generated Designer copy under `workPath/designer/<name>/`.
pub struct SourceSetsService<'a> {
    config: &'a AppConfig,
}

impl<'a> SourceSetsService<'a> {
    pub fn new(config: &'a AppConfig) -> Self {
        Self { config }
    }

    /// Return all Designer-format contexts that should be scanned and built.
    ///
    /// In `DESIGNER` mode this is simply each source-set resolved against the project base path.
    /// In `EDT` mode this returns the generated Designer copies: under the memory of the
    /// selected base (`workPath/infobases/<base>/designer/<name>`), because the copy and its
    /// version file describe the exchange with that base; a base the runner cannot remember
    /// and external artifacts keep `workPath/designer/<name>`.
    pub fn designer_contexts(&self) -> Vec<SourceSetContext> {
        let base_path = absolutize_path(&self.config.base_path);
        let work_path = absolutize_path(&self.config.work_path);
        let memory = self.base_memory();

        self.config
            .source_sets
            .iter()
            .map(|ss| {
                let memory = memory.as_ref().filter(|_| !ss.purpose.is_external());
                let path = match self.config.format {
                    SourceFormat::Designer => ss.root_in(&base_path),
                    SourceFormat::Edt => match memory {
                        Some(memory) => infobase_memory_dir(&work_path, &memory.key)
                            .join("designer")
                            .join(&ss.name),
                        None => work_path.join("designer").join(&ss.name),
                    },
                };
                self.designer_context(ss, path, memory, &base_path)
            })
            .collect()
    }

    fn designer_context(
        &self,
        source_set: &SourceSetConfig,
        path: PathBuf,
        memory: Option<&BaseMemory>,
        base_path: &Path,
    ) -> SourceSetContext {
        let context = SourceSetContext::new(
            &source_set.name,
            path,
            format!("designer-{}", source_set.name),
        );
        if source_set.purpose.is_external() {
            return context;
        }
        // An unrecognized address must never share memory.
        let Some(memory) = memory else {
            return context.without_memory();
        };
        let identity = format!(
            "{}; source={}; purpose={}; set={}",
            memory.address,
            source_identity(&source_set.root_in(base_path)),
            source_set.purpose.as_str(),
            source_set.name
        );
        context.with_infobase_memory(&memory.key, identity)
    }

    /// Context of a tool extension's sources: its hashes live under the memory of the
    /// selected base, like a source set's (`hashes/tools/<name>.redb`).
    pub fn tool_extension_context(&self, extension: &str, root: PathBuf) -> SourceSetContext {
        let context = SourceSetContext::new(
            format!("tool:{extension}"),
            root.clone(),
            format!("tool-{extension}-source"),
        );
        let Some(memory) = self.base_memory() else {
            return context.without_memory();
        };
        let identity = format!(
            "{}; source={}; tool-extension={extension}",
            memory.address,
            source_identity(&root),
        );
        context.with_tool_extension_memory(&memory.key, extension, identity)
    }

    /// Where the selected base is remembered: a declared base under its name, a base named
    /// by a connection string under a key derived from its address without credentials.
    /// `None` when the address is not recognized and so cannot be compared.
    fn base_memory(&self) -> Option<BaseMemory> {
        let base_path = absolutize_path(&self.config.base_path);
        let address = self.config.infobase_memory_address(&base_path)?;
        let key = match self.config.infobase_name.as_deref() {
            Some(name) => name.to_owned(),
            None => connection_memory_key(&address),
        };
        Some(BaseMemory { key, address })
    }

    /// Return EDT source-set contexts (only meaningful in `EDT` format).
    pub fn edt_contexts(&self) -> Vec<SourceSetContext> {
        if self.config.format != SourceFormat::Edt {
            return vec![];
        }
        let base_path = absolutize_path(&self.config.base_path);
        self.config
            .source_sets
            .iter()
            .map(|ss| {
                SourceSetContext::new(&ss.name, ss.root_in(&base_path), format!("edt-{}", ss.name))
            })
            .collect()
    }
    /// Analyze all provided contexts and return context-tagged outcomes.
    pub fn analyze_contexts(&self, contexts: &[SourceSetContext]) -> Vec<ContextAnalysis> {
        analyzer::analyze_contexts(contexts, &self.config.work_path)
    }
}

/// Каталог памяти выбранной базы и её адрес без учётных данных.
struct BaseMemory {
    key: String,
    address: String,
}

/// Канонический каталог исходников в привязке памяти: точное представление ОС.
fn source_identity(root: &Path) -> String {
    use crate::support::path::{nearest_existing_canonical_path, snapshot_path_identity};
    let canonical = nearest_existing_canonical_path(root).unwrap_or_else(|_| root.to_path_buf());
    snapshot_path_identity(&canonical)
}

/// Пути проекта приходят из загрузчика абсолютными; относительный — только у настроек,
/// собранных в коде, и тогда он считается от рабочего каталога процесса.
fn absolutize_path(path: &Path) -> PathBuf {
    crate::support::path::absolute_from_current_dir(path)
        .expect("project paths are absolute after loading, or the current directory is readable")
}

#[cfg(test)]
mod tests {
    use super::SourceSetsService;
    use crate::config::model::{
        AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use std::path::{Path, PathBuf};

    /// Проект с одним набором `main` в `src` и рабочим каталогом `work_path`.
    fn single_set_config(format: SourceFormat, work_path: &str) -> AppConfig {
        AppConfig {
            base_path: PathBuf::from("."),
            work_path: PathBuf::from(work_path),
            format,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: Some("main".to_owned()),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("src"),
            }],
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn designer_contexts_absolutize_relative_base_path() {
        let config = single_set_config(SourceFormat::Designer, "target/tmp-work");

        let service = SourceSetsService::new(&config);
        let contexts = service.designer_contexts();

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0].path().is_absolute());
        assert!(contexts[0].path().ends_with(Path::new("src")));
    }

    /// Снимок Конфигуратора набора EDT описывает обмен с базой и лежит под её памятью.
    #[test]
    fn an_edt_designer_copy_lies_under_the_base_memory() {
        let config = single_set_config(SourceFormat::Edt, "target/tmp-work");

        let service = SourceSetsService::new(&config);
        let contexts = service.designer_contexts();

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0]
            .path()
            .ends_with(Path::new("target/tmp-work/infobases/main/designer/main")));
    }

    /// Готовит в корне контекста модуль и служебный дочерний `build`, запоминает снимок и
    /// правит оба файла: изменение видно только в модуле.
    fn assert_root_named_like_a_service_dir_is_analyzed(config: &AppConfig) {
        use crate::change_detection::analyzer::{
            analyze_context, rescan_and_commit_full, AnalysisOutcome, ChangeKind,
        };
        let context = SourceSetsService::new(config).designer_contexts().remove(0);
        let root = context.path().to_path_buf();
        assert_eq!(root.file_name(), Some(std::ffi::OsStr::new("build")));
        let module = root.join("Module.bsl");
        let generated = root.join("build").join("Generated.bsl");
        std::fs::create_dir_all(generated.parent().expect("parent")).expect("set root");
        std::fs::write(&module, "Процедура А() КонецПроцедуры").expect("module");
        std::fs::write(&generated, "generated").expect("generated");
        rescan_and_commit_full(&context, &config.work_path).expect("snapshot");
        assert!(matches!(
            analyze_context(&context, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));

        std::fs::write(&module, "Процедура А() Возврат; КонецПроцедуры").expect("edit");
        std::fs::write(&generated, "regenerated").expect("regenerate");

        let Ok(AnalysisOutcome::Changes { changes, .. }) =
            analyze_context(&context, &config.work_path).outcome
        else {
            panic!("the edited module in the set root must be a change");
        };
        let changed: Vec<_> = changes
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        assert_eq!(changed, [(module, ChangeKind::Modified)]);
    }

    /// Набор, чей каталог называется как служебный (`build`), анализируется: служебные
    /// каталоги пропускаются только внутри набора.
    #[test]
    fn a_source_set_rooted_at_a_service_named_directory_is_analyzed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Designer, "unused");
        config.base_path = dir.path().to_path_buf();
        config.work_path = dir.path().join("work");
        config.source_sets[0].path = PathBuf::from("build");

        assert_root_named_like_a_service_dir_is_analyzed(&config);
    }

    /// Порождённая копия набора EDT `workPath/infobases/<база>/designer/build` анализируется так же.
    #[test]
    fn a_generated_designer_copy_named_build_is_analyzed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Edt, "unused");
        config.base_path = dir.path().to_path_buf();
        config.work_path = dir.path().join("work");
        config.source_sets[0].name = "build".to_owned();
        let generated_root = SourceSetsService::new(&config).designer_contexts()[0]
            .path()
            .to_path_buf();
        assert_eq!(
            generated_root,
            config.work_path.join("infobases/main/designer/build")
        );

        assert_root_named_like_a_service_dir_is_analyzed(&config);
    }

    /// Состояние анализа лежит под `workPath`, у каждого логического контекста набора своё:
    /// у набора EDT контекстов два, и хранилища у них разные.
    #[test]
    fn analysis_state_lies_under_the_work_path_by_logical_context() {
        let config = single_set_config(SourceFormat::Edt, "/tmp/work");
        let service = SourceSetsService::new(&config);

        let storages: Vec<PathBuf> = service
            .designer_contexts()
            .into_iter()
            .chain(service.edt_contexts())
            .filter_map(|context| context.storage_path(&config.work_path))
            .collect();

        let storage_root = config.work_path.join("hash-storages");
        assert_eq!(
            storages,
            [
                config.work_path.join("infobases/main/hashes/main.redb"),
                storage_root.join("edt-main.redb"),
            ]
        );
    }
    #[test]
    fn base_snapshots_remain_separate_and_reject_a_retargeted_base() {
        use crate::change_detection::analyzer::{
            analyze_context, rescan_and_commit_full, AnalysisOutcome, ChangeDetectionError,
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Designer, "unused");
        config.base_path = dir.path().to_path_buf();
        config.work_path = dir.path().join("work");
        std::fs::create_dir(dir.path().join("src")).expect("source");
        std::fs::write(dir.path().join("src/module.bsl"), "source").expect("write");
        let a = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        rescan_and_commit_full(&a, &config.work_path).expect("snapshot A");
        config.infobase_name = Some("B".to_owned());
        config.infobase.connection = "File=/tmp/B".to_owned();
        let b = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert!(matches!(
            analyze_context(&b, &config.work_path).outcome,
            Ok(AnalysisOutcome::Changes { .. })
        ));
        rescan_and_commit_full(&b, &config.work_path).expect("snapshot B");
        assert!(matches!(
            analyze_context(&a, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        config.infobase_name = Some("main".to_owned());
        let foreign = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        let error = analyze_context(&foreign, &config.work_path)
            .outcome
            .expect_err("retargeted base");
        assert!(
            matches!(error, ChangeDetectionError::ForeignMemory { .. }),
            "{error}"
        );
        // Выходы с глобальными ключами вызова дописывает сценарий: здесь их не из чего собрать.
        assert!(error.to_string().contains("belongs to"));
        assert!(!error.to_string().contains("--force"));
        assert!(error.to_string().contains("/tmp/ib"));
        rescan_and_commit_full(&foreign, &config.work_path).expect("explicit rebuild");
        assert!(matches!(
            analyze_context(&foreign, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        config.source_sets[0].path = PathBuf::from("moved");
        std::fs::rename(dir.path().join("src"), dir.path().join("moved")).expect("move source");
        let moved = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert!(analyze_context(&moved, &config.work_path).outcome.is_err());
    }

    /// База, названная строкой соединения, помнится по строке: память лежит под
    /// `workPath/infobases/` в каталоге с ключом из адреса без учётных данных, не
    /// пересекается с памятью объявленных баз, и следующий анализ продолжает с неё.
    /// Отсутствующий снимок — не ошибка, а пустой набор пропускается.
    #[test]
    fn an_ad_hoc_base_is_remembered_by_its_address_and_empty_sources_skip() {
        use crate::change_detection::analyzer::{analyze_context, commit_success, AnalysisOutcome};
        use crate::domain::source_set::connection_memory_key;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Designer, "unused");
        config.base_path = dir.path().to_path_buf();
        config.work_path = dir.path().join("work");
        config.infobase_name = None;
        std::fs::create_dir(dir.path().join("src")).expect("source");
        let context = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert!(matches!(
            analyze_context(&context, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        let address = config
            .infobase_memory_address(dir.path())
            .expect("a recognized address");
        let key = connection_memory_key(&address);
        assert!(!crate::config::model::is_infobase_name(&key));
        assert_eq!(
            context.storage_path(&config.work_path),
            Some(
                config
                    .work_path
                    .join("infobases")
                    .join(&key)
                    .join("hashes/main.redb")
            )
        );

        std::fs::write(dir.path().join("src/module.bsl"), "source").expect("write");
        let Ok(AnalysisOutcome::Changes { prepared, .. }) =
            analyze_context(&context, &config.work_path).outcome
        else {
            panic!("the first analysis sees existing files as added");
        };
        commit_success(&context, &config.work_path, &prepared).expect("remembered");

        config.infobase.connection = "File=/tmp/ib;Usr=alice;Pwd=secret".to_owned();
        let again = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_eq!(
            again.storage_path(&config.work_path),
            context.storage_path(&config.work_path)
        );
        assert!(matches!(
            analyze_context(&again, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        let bases: Vec<_> = std::fs::read_dir(config.work_path.join("infobases"))
            .expect("bases")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(bases, [std::ffi::OsString::from(&key)]);

        config.infobase.connection = "File=/tmp/other-ib".to_owned();
        let other = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_ne!(
            other.storage_path(&config.work_path),
            context.storage_path(&config.work_path)
        );
        assert!(matches!(
            analyze_context(&other, &config.work_path).outcome,
            Ok(AnalysisOutcome::Changes { .. })
        ));
    }

    /// Хеши исходников расширения-инструмента лежат под памятью базы рядом с хешами
    /// наборов, но в своём подкаталоге: набор с тем же именем файла с ними не делит.
    #[test]
    fn tool_extension_memory_lies_under_the_base_apart_from_source_sets() {
        let mut config = single_set_config(SourceFormat::Designer, "/tmp/work");
        config.source_sets[0].name = "client_mcp".to_owned();
        let service = SourceSetsService::new(&config);
        let tool = service.tool_extension_context("client_mcp", PathBuf::from("/tmp/tool"));
        let set = service.designer_contexts().remove(0);
        assert_eq!(
            tool.storage_path(&config.work_path),
            Some(
                config
                    .work_path
                    .join("infobases/main/hashes/tools/client_mcp.redb")
            )
        );
        assert_ne!(
            tool.storage_path(&config.work_path),
            set.storage_path(&config.work_path)
        );
        assert!(tool.storage_identity().expect("bound").contains("/tmp/ib"));
        assert_eq!(tool.version_file_copy_dir(&config.work_path), None);
        assert_eq!(tool.generation_file(&config.work_path), None);
        config.infobase_name = Some("other".to_owned());
        let other = SourceSetsService::new(&config)
            .tool_extension_context("client_mcp", PathBuf::from("/tmp/tool"));
        assert_ne!(
            other.storage_path(&config.work_path),
            tool.storage_path(&config.work_path)
        );
    }

    /// Строка, чей адрес раннер не распознаёт, сравнить не с чем: памяти у неё нет.
    #[test]
    fn an_unrecognized_address_keeps_no_memory() {
        let mut config = single_set_config(SourceFormat::Designer, "/tmp/work");
        config.infobase_name = None;
        config.infobase.connection = "Something=else".to_owned();
        let context = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_eq!(context.storage_path(&config.work_path), None);
    }

    #[test]
    fn edt_and_external_memory_is_shared_but_designer_identity_ignores_credentials() {
        let mut config = single_set_config(SourceFormat::Edt, "/tmp/work");
        let edt = SourceSetsService::new(&config).edt_contexts().remove(0);
        let designer = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        config.infobase.connection = "File=/tmp/ib;Usr=alice;Pwd=secret".to_owned();
        let with_credentials = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_eq!(
            designer.storage_identity(),
            with_credentials.storage_identity()
        );
        config.infobase_name = Some("other".to_owned());
        let other_edt = SourceSetsService::new(&config).edt_contexts().remove(0);
        assert_eq!(
            edt.storage_path(&config.work_path),
            other_edt.storage_path(&config.work_path)
        );
        config.source_sets[0].purpose = SourceSetPurpose::ExternalReports;
        let external = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert!(external.storage_identity().is_none());
        assert!(external.persists_snapshot());
    }
    #[test]
    fn standalone_snapshot_uses_gate_address_without_secrets_or_transport_settings() {
        let mut config = single_set_config(SourceFormat::Designer, "/tmp/work");
        config.infobase.connection.clear();
        config.infobase.standalone =
            Some(serde_yaml::from_str("gate: 'HOST:1543'\n").expect("standalone"));
        let before = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert!(before.persists_snapshot());
        assert!(before
            .storage_identity()
            .expect("identity")
            .contains("standalone:host:1543"));
        config.infobase.user = Some("alice".to_owned());
        config.infobase.password = Some("secret".to_owned());
        let after = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_eq!(before.storage_identity(), after.storage_identity());
        config
            .infobase
            .standalone
            .as_mut()
            .expect("standalone")
            .gate = "host:1544".to_owned();
        let moved = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_ne!(before.storage_identity(), moved.storage_identity());
    }
    #[cfg(unix)]
    #[test]
    fn distinct_non_utf8_source_roots_have_distinct_bindings() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Designer, "unused");
        config.base_path = dir.path().to_path_buf();
        config.work_path = dir.path().join("work");
        config.source_sets[0].path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xfe]));
        let original = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        config.source_sets[0].path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));
        let other = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_ne!(original.storage_identity(), other.storage_identity());
    }

    #[test]
    fn canonical_source_names_are_not_case_folded_for_memory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = single_set_config(SourceFormat::Designer, "unused");
        config.base_path = dir.path().to_path_buf();
        config.source_sets[0].path = PathBuf::from("source");
        let original = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        config.source_sets[0].path = PathBuf::from("SOURCE");
        let other = SourceSetsService::new(&config)
            .designer_contexts()
            .remove(0);
        assert_ne!(original.storage_identity(), other.storage_identity());
    }
}
