use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::change_detection::analyzer::{self, ContextAnalysis};
use crate::config::loader::normalize_connection_file_path;
use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig};
use crate::domain::source_set::SourceSetContext;
use crate::support::error::AppError;
use crate::support::path::nearest_existing_canonical_path;

/// Builds source contexts and owns their source/runtime snapshot binding.
pub struct SourceSetsService<'a> {
    config: &'a AppConfig,
}

impl<'a> SourceSetsService<'a> {
    pub fn new(config: &'a AppConfig) -> Self {
        Self { config }
    }

    /// Designer source contexts, including generated Designer output for EDT projects.
    #[cfg(test)]
    pub fn designer_contexts(&self) -> Result<Vec<SourceSetContext>, AppError> {
        self.config
            .source_sets
            .iter()
            .map(|source_set| self.designer_context(source_set))
            .collect()
    }

    /// Metadata-only path resolution does not require a live infobase binding.
    pub(crate) fn designer_path(&self, source_set: &SourceSetConfig) -> Result<PathBuf, AppError> {
        match self.config.format {
            SourceFormat::Designer => {
                Ok(absolutize_path(&self.config.base_path)?.join(&source_set.path))
            }
            SourceFormat::Edt => Ok(absolutize_path(&self.config.work_path)?
                .join("designer")
                .join(&source_set.name)),
        }
    }

    pub(crate) fn designer_context(
        &self,
        source_set: &SourceSetConfig,
    ) -> Result<SourceSetContext, AppError> {
        self.bind_context(
            SourceSetContext::new(
                &source_set.name,
                self.designer_path(source_set)?,
                format!("designer-{}", source_set.name),
            ),
            source_set,
            self.config.format,
        )
    }

    /// EDT source contexts, separate from their generated Designer load contexts.
    #[cfg(test)]
    pub fn edt_contexts(&self) -> Result<Vec<SourceSetContext>, AppError> {
        if self.config.format != SourceFormat::Edt {
            return Ok(vec![]);
        }
        self.config
            .source_sets
            .iter()
            .map(|source_set| self.edt_context(source_set))
            .collect()
    }

    pub(crate) fn edt_context(
        &self,
        source_set: &SourceSetConfig,
    ) -> Result<SourceSetContext, AppError> {
        self.bind_context(
            SourceSetContext::new(
                &source_set.name,
                absolutize_path(&self.config.base_path)?.join(&source_set.path),
                format!("edt-{}", source_set.name),
            ),
            source_set,
            SourceFormat::Edt,
        )
    }

    /// Bind ordinary and tool-extension contexts through the same identity owner.
    pub(crate) fn bind_context(
        &self,
        context: SourceSetContext,
        source_set: &SourceSetConfig,
        source_format: SourceFormat,
    ) -> Result<SourceSetContext, AppError> {
        let base_path = canonical_binding_path(&self.config.base_path)?;
        let context_path = canonical_binding_path(context.path())?;
        let source_path = canonical_binding_path(&base_path.join(&source_set.path))?;
        let connection = self.config.v8_connection();
        // Resolve against the actual working directory, just as the platform resolves /F.
        // The exact connection is also retained in the digest: uncertain aliases rebuild.
        // External artifact preparation/export is source-only; an IB is not its target.
        let uses_infobase = !source_set.purpose.is_external();
        let file_target = connection
            .file_path()
            .filter(|_| uses_infobase)
            .map(|path| {
                let working_directory = absolutize_path(Path::new("."))?;
                canonical_binding_path(Path::new(&normalize_connection_file_path(
                    path,
                    &working_directory,
                )))
            })
            .transpose()?;
        let dbms_target = self
            .config
            .infobase
            .dbms
            .as_ref()
            .filter(|_| uses_infobase)
            .map(|dbms| (&dbms.kind, &dbms.server, &dbms.name));
        let bytes = serde_json::to_vec(&(
            "source-runtime-binding-v1",
            context.name(),
            &context_path,
            &source_path,
            &base_path,
            &source_set.name,
            source_set.purpose,
            source_format,
            self.config.builder,
            uses_infobase.then_some(&self.config.infobase.connection),
            &file_target,
            dbms_target,
        ))
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to encode snapshot runtime binding: {error}"
            ))
        })?;
        Ok(context.with_runtime_binding(format!("{:x}", Sha256::digest(bytes))))
    }

    pub fn analyze_contexts(&self, contexts: &[SourceSetContext]) -> Vec<ContextAnalysis> {
        analyzer::analyze_contexts(contexts, &self.config.work_path)
    }
}

fn absolutize_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to resolve current working directory: {error}"
            ))
        })
}

fn canonical_binding_path(path: &Path) -> Result<PathBuf, AppError> {
    nearest_existing_canonical_path(&absolutize_path(path)?).map_err(|error| {
        AppError::Runtime(format!(
            "failed to resolve snapshot runtime path '{}': {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::SourceSetsService;
    use crate::config::model::{
        AppConfig, BuildConfig, BuilderBackend, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use std::path::Path;

    #[test]
    fn designer_contexts_absolutize_relative_base_path() {
        let config = AppConfig {
            base_path: std::path::PathBuf::from("."),
            work_path: std::path::PathBuf::from("target/tmp-work"),
            execution_timeout: 300_000,
            format: SourceFormat::Designer,
            builder: BuilderBackend::Designer,
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: std::path::PathBuf::from("src"),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        };

        let service = SourceSetsService::new(&config);
        let contexts = service.designer_contexts().expect("contexts");

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0].path().is_absolute());
        assert!(contexts[0].path().ends_with(Path::new("src")));
    }

    #[test]
    fn edt_designer_contexts_use_nested_designer_directory() {
        let config = AppConfig {
            base_path: std::path::PathBuf::from("."),
            work_path: std::path::PathBuf::from("target/tmp-work"),
            execution_timeout: 300_000,
            format: SourceFormat::Edt,
            builder: BuilderBackend::Designer,
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: std::path::PathBuf::from("src"),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        };

        let service = SourceSetsService::new(&config);
        let contexts = service.designer_contexts().expect("contexts");

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0]
            .path()
            .ends_with(Path::new("target/tmp-work/designer/main")));
    }

    fn binding_config(root: &Path) -> AppConfig {
        AppConfig {
            base_path: root.join("base"),
            work_path: root.join("work"),
            execution_timeout: 300_000,
            format: SourceFormat::Designer,
            builder: BuilderBackend::Designer,
            infobase: crate::config::model::InfobaseConfig::file(format!(
                "File={}",
                root.join("ib").display()
            )),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: "src".into(),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn binding(config: &AppConfig) -> String {
        SourceSetsService::new(config)
            .designer_contexts()
            .expect("contexts")[0]
            .runtime_binding()
            .expect("bound production context")
            .to_owned()
    }

    #[test]
    fn runtime_binding_tracks_source_and_target_without_changing_storage_slot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = binding_config(dir.path());
        let before = binding(&config);
        let mut other_source = config.clone();
        other_source.source_sets[0].path = "other-src".into();
        let mut other_ib = config.clone();
        other_ib.infobase.connection = format!("File={}", dir.path().join("other-ib").display());
        let mut other_purpose = config.clone();
        other_purpose.source_sets[0].purpose = SourceSetPurpose::Extension;
        let mut other_builder = config.clone();
        other_builder.builder = BuilderBackend::Ibcmd;
        let original_slot = SourceSetsService::new(&config)
            .designer_contexts()
            .expect("contexts")[0]
            .storage_path(&config.work_path);
        for changed in [other_source, other_ib, other_purpose, other_builder] {
            assert_ne!(before, binding(&changed));
            let context = SourceSetsService::new(&changed)
                .designer_contexts()
                .expect("contexts")
                .remove(0);
            assert_eq!(original_slot, context.storage_path(&config.work_path));
        }
        assert_eq!(before, binding(&config));
    }

    #[test]
    fn runtime_binding_ignores_separate_authentication_fields_and_source_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = binding_config(dir.path());
        std::fs::create_dir_all(config.base_path.join("src")).expect("source");
        let before = binding(&config);
        config.infobase.user = Some("user".to_owned());
        config.infobase.password = Some("secret".to_owned());
        std::fs::write(config.base_path.join("src/Module.bsl"), "changed").expect("source bytes");
        assert_eq!(before, binding(&config));
    }

    #[test]
    fn generated_designer_binding_includes_original_edt_source_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = binding_config(dir.path());
        config.format = SourceFormat::Edt;
        let before = binding(&config);
        let original_generated_path = SourceSetsService::new(&config)
            .designer_contexts()
            .expect("contexts")[0]
            .path()
            .to_path_buf();
        config.source_sets[0].path = "other-edt-src".into();
        assert_ne!(before, binding(&config));
        assert_eq!(
            original_generated_path,
            SourceSetsService::new(&config)
                .designer_contexts()
                .expect("contexts")[0]
                .path()
        );
    }

    #[test]
    fn runtime_binding_includes_effective_ibcmd_dbms_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = binding_config(dir.path());
        config.builder = BuilderBackend::Ibcmd;
        config.infobase.connection = "Srvr=server;Ref=database".to_owned();
        config.infobase.dbms = Some(crate::config::model::InfobaseDbmsConfig {
            kind: Some("PostgreSQL".to_owned()),
            server: Some("db-server".to_owned()),
            name: Some("db-a".to_owned()),
            ..Default::default()
        });
        let before = binding(&config);
        config.infobase.dbms.as_mut().expect("dbms").name = Some("db-b".to_owned());
        assert_ne!(before, binding(&config));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_binding_detects_retargeted_infobase_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = binding_config(dir.path());
        let first = dir.path().join("ib-first");
        let second = dir.path().join("ib-second");
        let link = dir.path().join("ib");
        std::fs::create_dir(&first).expect("first ib");
        std::fs::create_dir(&second).expect("second ib");
        symlink(&first, &link).expect("link");
        let before = binding(&config);
        std::fs::remove_file(&link).expect("unlink");
        symlink(&second, &link).expect("retarget");
        assert_ne!(before, binding(&config));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_binding_detects_retargeted_quoted_file_infobase() {
        use std::os::unix::fs::symlink;
        for quote in ['"', '\''] {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut config = binding_config(dir.path());
            let first = dir.path().join("ib first");
            let second = dir.path().join("ib second");
            let link = dir.path().join("ib alias");
            std::fs::create_dir(&first).expect("first ib");
            std::fs::create_dir(&second).expect("second ib");
            symlink(&first, &link).expect("link");
            config.infobase.connection = format!("File={quote}{}{quote};", link.display());
            std::fs::create_dir_all(config.base_path.join("src")).expect("source");
            let yaml_path = dir.path().join("v8project.yaml");
            let yaml = format!(
                "workPath: {}\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: {}\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: base/src\n",
                config.work_path.display(),
                serde_json::to_string(&config.infobase.connection).expect("connection YAML string"),
            );
            std::fs::write(&yaml_path, yaml).expect("config YAML");
            let loaded = crate::config::loader::load_config(
                Some(yaml_path.to_str().expect("config path")),
                None,
            )
            .expect("loaded config");
            let loaded_before = binding(&loaded);
            let before = binding(&config);
            std::fs::remove_file(&link).expect("unlink");
            symlink(&second, &link).expect("retarget");
            assert_ne!(
                before,
                binding(&config),
                "quoted File alias must track actual infobase target"
            );
            assert_ne!(
                loaded_before,
                binding(&loaded),
                "loaded config must track actual infobase target"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_binding_detects_retargeted_source_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = binding_config(dir.path());
        std::fs::create_dir(&config.base_path).expect("base");
        let first = dir.path().join("source-first");
        let second = dir.path().join("source-second");
        let link = config.base_path.join("src");
        std::fs::create_dir(&first).expect("first source");
        std::fs::create_dir(&second).expect("second source");
        symlink(&first, &link).expect("link");
        let before = binding(&config);
        std::fs::remove_file(&link).expect("unlink");
        symlink(&second, &link).expect("retarget");
        assert_ne!(before, binding(&config));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_binding_propagates_dangling_source_symlink_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = binding_config(dir.path());
        std::fs::create_dir(&config.base_path).expect("base");
        std::os::unix::fs::symlink(dir.path().join("missing"), config.base_path.join("src"))
            .expect("dangling source");
        assert!(SourceSetsService::new(&config).designer_contexts().is_err());
    }
    #[cfg(unix)]
    #[test]
    fn external_build_remains_source_only_with_unavailable_infobase() {
        for (purpose, root_tag) in [
            (
                SourceSetPurpose::ExternalDataProcessors,
                "ExternalDataProcessor",
            ),
            (SourceSetPurpose::ExternalReports, "ExternalReport"),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut config = binding_config(dir.path());
            config.source_sets[0].purpose = purpose;
            let source = config.base_path.join("src");
            std::fs::create_dir_all(&source).expect("source");
            std::fs::write(
                source.join("Example.xml"),
                format!("<{root_tag}><Properties><Name>Example</Name></Properties></{root_tag}>"),
            )
            .expect("descriptor");
            let alias = dir.path().join("unavailable-ib");
            std::os::unix::fs::symlink(dir.path().join("absent"), &alias).expect("dangling IB");
            config.infobase.connection = format!("File=\"{}\"", alias.display());
            let before = binding(&config);
            let result = crate::use_cases::build_project::run_build(
                &config,
                &crate::use_cases::request::BuildRequest {
                    full_rebuild: false,
                    source_set: Some(config.source_sets[0].name.clone()),
                    dry_run: false,
                },
            )
            .expect("source-only external build");
            assert!(result.ok);
            assert_eq!(result.steps.len(), 1);
            assert_eq!(
                result.steps[0].mode,
                crate::domain::build::BuildMode::Skipped
            );
            config.infobase.connection = "File=/another/unavailable/ib".to_owned();
            assert_eq!(
                before,
                binding(&config),
                "IB changes must not affect source-only contexts"
            );
        }
    }
}
