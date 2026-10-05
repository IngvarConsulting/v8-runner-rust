use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::{self, ContextAnalysis};
use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig};
use crate::domain::source_set::SourceSetContext;

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
    /// In `EDT` mode (Wave 2) this returns the generated Designer copies in
    /// `workPath/designer`.
    pub fn designer_contexts(&self) -> Vec<SourceSetContext> {
        let base_path = absolutize_path(&self.config.base_path);
        let work_path = absolutize_path(&self.config.work_path);

        match self.config.format {
            SourceFormat::Designer => self
                .config
                .source_sets
                .iter()
                .map(|ss| self.designer_context(ss, ss.root_in(&base_path)))
                .collect(),

            SourceFormat::Edt => self
                .config
                .source_sets
                .iter()
                .map(|ss| {
                    // Generated Designer copy lives at workPath/designer/<name>/
                    let path = work_path.join("designer").join(&ss.name);
                    self.designer_context(ss, path)
                })
                .collect(),
        }
    }

    fn designer_context(&self, source_set: &SourceSetConfig, path: PathBuf) -> SourceSetContext {
        use crate::support::path::{nearest_existing_canonical_path, snapshot_path_identity};
        let context = SourceSetContext::new(
            &source_set.name,
            path,
            format!("designer-{}", source_set.name),
        );
        if source_set.purpose.is_external() {
            return context;
        }
        let base_path = absolutize_path(&self.config.base_path);
        // An unrecognized address, or an ad hoc base without a name, must never share memory.
        let (Some(address), Some(infobase)) = (
            self.config.infobase_memory_address(&base_path),
            self.config.infobase_name.as_deref(),
        ) else {
            return context.without_memory();
        };
        let original = source_set.root_in(&base_path);
        let original = nearest_existing_canonical_path(&original).unwrap_or(original);
        let identity = format!(
            "{}; source={}; purpose={}; set={}",
            address,
            snapshot_path_identity(&original),
            source_set.purpose.as_str(),
            source_set.name
        );
        context.with_infobase_memory(infobase, identity)
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
        AppConfig, BuildConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
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
            build: BuildConfig::default(),
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

    #[test]
    fn edt_designer_contexts_use_nested_designer_directory() {
        let config = single_set_config(SourceFormat::Edt, "target/tmp-work");

        let service = SourceSetsService::new(&config);
        let contexts = service.designer_contexts();

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0]
            .path()
            .ends_with(Path::new("target/tmp-work/designer/main")));
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

    /// Порождённая копия набора EDT `workPath/designer/build` анализируется так же.
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
            config.work_path.join("designer").join("build")
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
        assert!(error.to_string().contains("full pull"));
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

    #[test]
    fn ad_hoc_analysis_never_reads_or_writes_memory_and_empty_sources_skip() {
        use crate::change_detection::analyzer::{
            analyze_context, commit_success, rescan_and_commit_full, AnalysisOutcome,
        };
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
        std::fs::write(dir.path().join("src/module.bsl"), "source").expect("write");
        rescan_and_commit_full(&context, &config.work_path).expect("no-op");
        assert!(!config.work_path.exists());
        assert_eq!(context.storage_path(&config.work_path), None);
        // An ad hoc base never touches the old shared file, even when it is unreadable.
        let legacy = config
            .work_path
            .join("hash-storages")
            .join(format!("designer-{}.redb", context.name()));
        std::fs::create_dir_all(&legacy).expect("unreadable old memory");
        let Ok(AnalysisOutcome::Changes { prepared, .. }) =
            analyze_context(&context, &config.work_path).outcome
        else {
            panic!("ordinary added files");
        };
        commit_success(&context, &config.work_path, &prepared).expect("no-op");
        assert!(legacy.is_dir());
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
