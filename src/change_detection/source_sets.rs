use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::{self, ContextAnalysis};
use crate::config::model::{AppConfig, SourceFormat};
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
                .map(|ss| {
                    let path = if ss.path.is_absolute() {
                        ss.path.clone()
                    } else {
                        base_path.join(&ss.path)
                    };
                    SourceSetContext::new(&ss.name, path, format!("designer-{}", ss.name))
                })
                .collect(),

            SourceFormat::Edt => self
                .config
                .source_sets
                .iter()
                .map(|ss| {
                    // Generated Designer copy lives at workPath/designer/<name>/
                    let path = work_path.join("designer").join(&ss.name);
                    SourceSetContext::new(&ss.name, path, format!("designer-{}", ss.name))
                })
                .collect(),
        }
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
                let path = if ss.path.is_absolute() {
                    ss.path.clone()
                } else {
                    base_path.join(&ss.path)
                };
                SourceSetContext::new(&ss.name, path, format!("edt-{}", ss.name))
            })
            .collect()
    }
    /// Analyze all provided contexts and return context-tagged outcomes.
    pub fn analyze_contexts(&self, contexts: &[SourceSetContext]) -> Vec<ContextAnalysis> {
        analyzer::analyze_contexts(contexts, &self.config.work_path)
    }
}

fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir()
        .expect("failed to resolve current working directory")
        .join(path)
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
            infobase_name: None,
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
            .map(|context| context.storage_path(&config.work_path))
            .collect();

        let storage_root = config.work_path.join("hash-storages");
        assert_eq!(
            storages,
            [
                storage_root.join("designer-main.redb"),
                storage_root.join("edt-main.redb"),
            ]
        );
    }
}
