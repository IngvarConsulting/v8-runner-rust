//! `pull --all`: наборы по составу базы.
//!
//! Команда спрашивает базу, какие расширения в ней установлены (замер #187: у каждого
//! исполнителя свой вызов, ответ разбирается по структуре, а не по тексту сообщений), и
//! обходит пакеты конфигурации порядком [`SourceSetInventory::configuration_packages`]:
//! сперва наборы проекта, затем по набору `src/ext/<Name>` на каждое расширение без набора.
//! Каждый набор выгружается тем же сценарием, что `pull <SET>`, со сторожем каталога и
//! памятью базы; объявление нового набора дописывается в `v8project.yaml` после его удачной
//! выгрузки, так что отказ посреди обхода не оставляет в проекте набора без содержимого.

use std::collections::HashSet;

use super::*;
use crate::config::model::SourceSetConfig;
use crate::domain::capability::Operation;
use crate::domain::config_init::ConfigInitSourceSet;
use crate::domain::dump::{NotDeclaredExtension, PullAllResult};
use crate::platform::extension_inventory::is_windows_device_name;
use crate::use_cases::config_init::{declare_source_sets, with_declared_source_sets};
use crate::use_cases::extension_identity::extension_name_key;
use crate::use_cases::provider_selection::SelectedProvider;
use crate::use_cases::request::PullAllRequest;
use crate::use_cases::result::UseCaseError;
use crate::use_cases::set_walk;
use crate::use_cases::source_inventory::{comparable_path, paths_overlap};

/// Каталог, под которым `pull --all` заводит набор расширения: `src/ext/<Name>` от
/// `basePath`, как пути всех наборов.
const DECLARED_EXTENSION_ROOT: &str = "src/ext";

pub fn execute_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PullAllRequest,
) -> UseCaseResult<PullAllResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        dry_run = request.dry_run,
        "executing pull --all"
    );
    stamp_dispatch(
        crate::use_cases::provider_selection::stamp_session(
            run_all(context, config, request),
            context,
        ),
        context.work(),
    )
}

fn run_all(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PullAllRequest,
) -> UseCaseResult<PullAllResult> {
    let started = Instant::now();
    let mut result = PullAllResult {
        provider: None,
        ok: false,
        provider_dispatched: false,
        declared: None,
        not_installed: Vec::new(),
        not_declared: Vec::new(),
        if_installed: Vec::new(),
        sets: Vec::new(),
        duration_ms: 0,
        message: None,
    };

    if let Some(error) = validate_supported_matrix(config) {
        return Err(set_walk::fail(error, result, started));
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let selected =
        match crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Dump)
        {
            Ok(selected) => selected,
            Err((error, receipt)) => {
                result.provider = Some(receipt);
                return Err(set_walk::fail(error, result, started));
            }
        };
    result.provider = Some(selected.receipt.clone());

    if request.dry_run {
        let walker = Walker {
            context,
            config,
            request,
        };
        return walker.preview(&selected, result, started);
    }

    let binary = selected.location.as_ref().map(|found| found.path.as_path());
    let installed = match crate::use_cases::installed_extensions::read_installed_extensions(
        context,
        config,
        Operation::Dump,
        selected.provider,
        binary,
        &utilities,
    ) {
        Ok(installed) => installed,
        Err(error) => {
            return Err(set_walk::fail_after_possible_work(
                error,
                result,
                started,
                context.work(),
            ))
        }
    };
    // Состав базы прочитан: дальше `declared` называет объявленное, пусть и ничего.
    result.declared = Some(Vec::new());
    let walk = match plan_walk(config, &installed).and_then(|walk| {
        // Проект с новыми наборами проверяется до первой выгрузки и тем же валидатором, что
        // проект при загрузке: выгруженное не должно остаться без объявления, а объявление —
        // ломать следующий запуск.
        crate::config::validate::validate_with_declared_source_sets(config, &walk.declared_configs())
            .map_err(|error| {
                AppError::Validation(format!(
                    "the source-sets pull --all would declare leave the project invalid, so nothing was pulled: {error}"
                ))
            })?;
        let text = std::fs::read_to_string(&request.project_file).map_err(|error| {
            AppError::Runtime(format!(
                "failed to read project file '{}': {error}",
                request.project_file.display()
            ))
        })?;
        with_declared_source_sets(&text, &request.project_file, &walk.declared)?;
        Ok(walk)
    }) {
        Ok(walk) => walk,
        Err(error) => return Err(set_walk::fail(error, result, started)),
    };
    result.not_installed = walk
        .not_installed
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    result.not_declared = walk.not_declared.clone();

    // Наборы проекта и объявляемые идут одним обходом: в настройках этой команды новые
    // наборы уже есть, на диске — появляются после своей выгрузки.
    let mut walked = config.clone();
    walked.source_sets.extend(walk.declared_configs());
    let walker = Walker {
        context,
        config: &walked,
        request,
    };
    let mut declared = Vec::with_capacity(walk.declared.len());
    for name in &walk.existing {
        if let Err(error) = walker.pull(name, SetKind::Project, &mut result) {
            result.declared = Some(declared);
            return Err(set_walk::fail(error, result, started));
        }
    }
    for set in &walk.declared {
        let pulled = walker
            .pull(&set.name, SetKind::Declared, &mut result)
            .and_then(|()| {
                declare_source_sets(&request.project_file, std::slice::from_ref(set))
                    .map_err(UseCaseError::from)
            });
        if let Err(error) = pulled {
            result.declared = Some(declared);
            return Err(set_walk::fail(error, result, started));
        }
        declared.push(set.clone());
    }
    result.declared = Some(declared);
    result.ok = true;
    Ok(set_walk::finish(result, started))
}

/// Чем выгружается набор обхода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetKind {
    /// Набор проекта: как `pull <SET>` — по изменившемуся, с `--force` полной заменой.
    Project,
    /// Объявляемый набор: каталога у него ещё нет, и выгрузка полная, как первая. Сторож
    /// спрашивается так же — каталог, оставшийся от прерванного прогона, без согласия не
    /// заменяется, — но в совете нет `pull <SET> --force`: набора в проекте ещё нет.
    Declared,
}

/// Общее для каждой выгрузки обхода: сценарий, настройки с объявляемыми наборами и запрос.
struct Walker<'a> {
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    request: &'a PullAllRequest,
}

impl Walker<'_> {
    /// Превью платформу не запускает, поэтому состава базы не знает: какие наборы объявит
    /// настоящий прогон и каких расширений проекта в базе нет, выясняется только им. Превью
    /// выгрузки называет наборы, которые прогон выгрузит при любом составе, — основную
    /// конфигурацию; наборы расширений проекта названы в `if_installed`, а объявляемые —
    /// шаблоном `src/ext/<Name>`.
    fn preview(
        &self,
        selected: &SelectedProvider,
        mut result: PullAllResult,
        started: Instant,
    ) -> UseCaseResult<PullAllResult> {
        for (source_set, extension) in SourceSetInventory::new(self.config).configuration_packages()
        {
            if extension.is_some() {
                result.if_installed.push(source_set.name.clone());
            } else if let Err(error) = self.pull(&source_set.name, SetKind::Project, &mut result) {
                return Err(set_walk::fail(error, result, started));
            }
        }
        result.message = Some(format!(
            "would read the extensions installed in the infobase via {}, pull each extension set of the project the infobase has and name the others as not installed, and, for each extension without a set, pull it into `{DECLARED_EXTENSION_ROOT}/<Name>` and then declare that set in '{}'; which sets would be declared is known only once the infobase is read; nothing read, nothing written",
            provider_label(selected),
            self.request.project_file.display()
        ));
        result.ok = true;
        Ok(set_walk::finish(result, started))
    }

    /// Выгружает один набор сценарием `pull <SET>` и кладёт его ответ в обход; отказ набора
    /// останавливает обход.
    fn pull(
        &self,
        source_set: &str,
        kind: SetKind,
        result: &mut PullAllResult,
    ) -> Result<(), UseCaseError> {
        let set_request = DumpArgs {
            mode: if self.request.discard_uncommitted || kind == SetKind::Declared {
                DumpModeRequest::Full
            } else {
                DumpModeRequest::Incremental
            },
            source_set: Some(source_set.to_owned()),
            extension: None,
            objects: Vec::new(),
            dry_run: self.request.dry_run,
            discard_uncommitted: self.request.discard_uncommitted,
            force_way_out: match kind {
                SetKind::Project => ForceWayOut::PullForce,
                SetKind::Declared => ForceWayOut::Undeclared,
            },
        };
        set_walk::collect_set(
            &mut result.sets,
            super::execute(self.context, self.config, &set_request),
        )
    }
}

/// Что обойти: наборы проекта в порядке пакетов и наборы, которые надо объявить.
#[derive(Debug, PartialEq, Eq)]
struct Walk<'a> {
    /// Пакеты проекта в порядке обхода: основная конфигурация и расширения, которые в базе
    /// есть.
    existing: Vec<&'a str>,
    /// Наборы для расширений базы без набора, по имени.
    declared: Vec<ConfigInitSourceSet>,
    /// Наборы расширений проекта, которых в базе нет.
    not_installed: Vec<&'a str>,
    /// Расширения без набора, которым набор не объявить, с причиной.
    not_declared: Vec<NotDeclaredExtension>,
}

impl Walk<'_> {
    /// Объявляемые наборы так, как их прочтёт следующий запуск.
    fn declared_configs(&self) -> Vec<SourceSetConfig> {
        self.declared
            .iter()
            .map(|declared| SourceSetConfig {
                name: declared.name.clone(),
                purpose: SourceSetPurpose::Extension,
                path: PathBuf::from(&declared.path),
            })
            .collect()
    }
}

/// Сопоставляет состав базы с наборами проекта ([`SourceSetInventory::installed_packages`]:
/// по имени расширения, без регистра, со сторожем #218) и планирует наборы для расширений
/// базы без набора: объявить второй набор для расширения, которое набор уже держит, значило
/// бы раздвоить его. Расширение-инструмент клиентского MCP (`tools.client_mcp.extension`)
/// раннер ставит сам, и набором оно не объявляется.
fn plan_walk<'a>(config: &'a AppConfig, installed: &[String]) -> Result<Walk<'a>, AppError> {
    let key = extension_name_key;
    let inventory = SourceSetInventory::new(config);
    let packages = inventory.installed_packages(installed)?;
    let existing = packages
        .present
        .iter()
        .map(|(source_set, _)| source_set.name.as_str())
        .collect::<Vec<_>>();
    let not_installed = packages
        .not_installed
        .iter()
        .map(|source_set| source_set.name.as_str())
        .collect::<Vec<_>>();
    let claimed = inventory
        .configuration_packages()
        .into_iter()
        .filter_map(|(_, extension)| extension.map(key))
        .collect::<HashSet<_>>();

    let tool = config
        .tools
        .client_mcp
        .extension
        .as_ref()
        .map(|tool| key(&tool.name));
    let set_roots = config
        .source_sets
        .iter()
        .map(|source_set| {
            (
                source_set,
                comparable_path(&source_set.root_in(&config.base_path)),
            )
        })
        .collect::<Vec<_>>();
    let mut declared = Vec::new();
    let mut not_declared = Vec::new();
    for name in installed {
        if claimed.contains(&key(name)) || tool.as_deref() == Some(key(name).as_str()) {
            continue;
        }
        // Имя устройства — верный идентификатор 1С, но каталогом `src/ext/<Name>` в Windows
        // не стать: объявление пропускается и называется, прочие наборы выгружаются.
        if is_windows_device_name(name) {
            not_declared.push(NotDeclaredExtension {
                name: name.clone(),
                reason: format!(
                    "directory '{DECLARED_EXTENSION_ROOT}/{name}' is impossible on Windows, where '{name}' is a device name: declare the set by hand under another path"
                ),
            });
            continue;
        }
        if let Some(taken) = config
            .source_sets
            .iter()
            .find(|source_set| key(&source_set.name) == key(name))
        {
            return Err(AppError::Validation(format!(
                "the infobase has extension '{name}', and its name is taken by the {} source-set '{}' in the project: an extension source-set cannot be declared under it; rename that source-set",
                taken.purpose.as_str(),
                taken.name
            )));
        }
        // Каталог нового набора не должен лежать внутри каталога набора проекта или вмещать его:
        // полная выгрузка внешнего каталога заменила бы вложенный целиком. Совпадающий
        // каталог — отказ проверки плана, а не пропуск, кто бы ни стоял в проекте раньше.
        let path = format!("{DECLARED_EXTENSION_ROOT}/{name}");
        let root = comparable_path(&config.base_path.join(&path));
        let overlapping = (!set_roots.iter().any(|(_, other)| *other == root))
            .then(|| {
                set_roots
                    .iter()
                    .find(|(_, other)| paths_overlap(&root, other))
            })
            .flatten();
        if let Some((overlapping, _)) = overlapping {
            not_declared.push(NotDeclaredExtension {
                name: name.clone(),
                reason: format!(
                    "directory '{path}' overlaps the directory '{}' of source-set '{}', and a full pull of one would replace the other: declare the set by hand under another path",
                    overlapping.path.display(),
                    overlapping.name
                ),
            });
            continue;
        }
        declared.push(ConfigInitSourceSet {
            name: name.clone(),
            source_type: SourceSetPurpose::Extension.as_str().to_owned(),
            path,
        });
    }
    declared.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Walk {
        existing,
        declared,
        not_installed,
        not_declared,
    })
}

fn provider_label(selected: &SelectedProvider) -> String {
    match &selected.location {
        Some(found) => found.path.display().to_string(),
        None => "the attached Designer agent".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_walk, Walk};
    use crate::config::model::{
        AppConfig, InfobaseConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolExtensionArtifactConfig, ToolExtensionConfig, ToolExtensionInput, ToolsConfig,
    };
    use crate::domain::config_init::ConfigInitSourceSet;

    fn set(name: &str, purpose: SourceSetPurpose, path: &str) -> SourceSetConfig {
        SourceSetConfig {
            name: name.to_owned(),
            purpose,
            path: path.into(),
        }
    }

    fn config_in(base_path: &std::path::Path, source_sets: Vec<SourceSetConfig>) -> AppConfig {
        AppConfig {
            base_path: base_path.join("base"),
            work_path: base_path.join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets,
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn config(source_sets: Vec<SourceSetConfig>) -> AppConfig {
        config_in(
            &std::env::temp_dir().join("v8-runner-pull-all-plan"),
            source_sets,
        )
    }

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn declared(name: &str) -> ConfigInitSourceSet {
        ConfigInitSourceSet {
            name: name.to_owned(),
            source_type: "EXTENSION".to_owned(),
            path: format!("src/ext/{name}"),
        }
    }

    /// Наборы проекта идут первыми в порядке пакетов, расширения без набора объявляются под
    /// `src/ext/<Name>` по имени; набор, чьего расширения в базе нет, не выгружается, а имя
    /// сравнивается без регистра.
    #[test]
    fn the_walk_keeps_project_sets_and_declares_the_rest() {
        let config = config(vec![
            set("old", SourceSetPurpose::Extension, "exts/old"),
            set("main", SourceSetPurpose::Configuration, "src/cf"),
            set("gone", SourceSetPurpose::Extension, "exts/gone"),
            set("reports", SourceSetPurpose::ExternalReports, "erf"),
        ]);

        let walk = plan_walk(&config, &installed(&["Новое", "Old", "Second"])).expect("walk");

        assert_eq!(
            walk,
            Walk {
                existing: vec!["main", "old"],
                declared: vec![declared("Second"), declared("Новое")],
                not_installed: vec!["gone"],
                not_declared: Vec::new(),
            }
        );
    }

    /// Расширение с именем устройства Windows набора не получает: объявление пропущено и
    /// названо с причиной, остальные объявляются.
    #[test]
    fn a_windows_device_name_is_named_not_declared() {
        let config = config(vec![set("main", SourceSetPurpose::Configuration, "src/cf")]);

        let walk = plan_walk(&config, &installed(&["Aux", "Sales"])).expect("walk");

        assert_eq!(walk.declared, vec![declared("Sales")]);
        assert_eq!(walk.not_declared.len(), 1, "{walk:?}");
        assert_eq!(walk.not_declared[0].name, "Aux");
        assert!(
            walk.not_declared[0].reason.contains("device name"),
            "{walk:?}"
        );
    }

    /// Каталог `src/ext/<Name>`, вложенный в каталог набора проекта или вмещающий его, не
    /// объявляется: полная выгрузка внешнего набора заменила бы вложенный.
    #[test]
    fn a_directory_overlapping_a_project_set_is_named_not_declared() {
        let outer = config(vec![
            set("main", SourceSetPurpose::Configuration, "src/cf"),
            set("ext", SourceSetPurpose::Extension, "src/ext"),
        ]);

        let walk = plan_walk(&outer, &installed(&["Ext", "Sales"])).expect("walk");

        assert!(walk.declared.is_empty(), "{walk:?}");
        assert_eq!(walk.not_declared.len(), 1, "{walk:?}");
        assert_eq!(walk.not_declared[0].name, "Sales");
        assert!(
            walk.not_declared[0].reason.contains("source-set 'ext'"),
            "{walk:?}"
        );

        let inner = config(vec![
            set("main", SourceSetPurpose::Configuration, "src/cf"),
            set("deep", SourceSetPurpose::Extension, "src/ext/Sales/inner"),
        ]);
        let walk = plan_walk(&inner, &installed(&["Deep", "Other", "Sales"])).expect("walk");
        assert_eq!(walk.declared, vec![declared("Other")]);
        assert_eq!(walk.not_declared.len(), 1, "{walk:?}");
        let reason = &walk.not_declared[0].reason;
        assert!(reason.contains("source-set 'deep'"), "{walk:?}");
        assert!(reason.contains("declare the set by hand"), "{walk:?}");
    }

    /// Совпадающий каталог остаётся отказом проверки плана, даже если раньше в проекте стоит
    /// набор, который каталог нового набора вмещает.
    #[test]
    fn an_equal_directory_is_left_to_the_plan_check_despite_an_earlier_overlap() {
        let config = config(vec![
            set("main", SourceSetPurpose::Configuration, "src"),
            set("legacy", SourceSetPurpose::Extension, "src/ext/Sales"),
        ]);

        let walk = plan_walk(&config, &installed(&["Legacy", "Sales"])).expect("walk");

        assert_eq!(walk.declared, vec![declared("Sales")]);
        assert!(walk.not_declared.is_empty(), "{walk:?}");
    }

    /// Имя расширения, занятое набором другого назначения, — отказ до выгрузки.
    #[test]
    fn a_name_taken_by_a_set_of_another_purpose_is_refused() {
        let config = config(vec![set("main", SourceSetPurpose::Configuration, "src/cf")]);

        let taken_name = plan_walk(&config, &installed(&["MAIN"])).expect_err("name");

        assert!(taken_name.to_string().contains("'main'"), "{taken_name}");
    }

    /// Расширение-инструмент клиентского MCP ставит раннер: набором оно не объявляется,
    /// в каком бы регистре его ни назвала база.
    #[test]
    fn the_client_mcp_tool_extension_is_not_declared() {
        let mut config = config(vec![set("main", SourceSetPurpose::Configuration, "src/cf")]);
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Artifact(ToolExtensionArtifactConfig {
                path: "client-mcp.cfe".into(),
            }),
        });

        let walk = plan_walk(&config, &installed(&["Client_MCP", "Sales"])).expect("walk");

        assert_eq!(walk.declared, vec![declared("Sales")]);
    }

    /// Набор `my-ext`, исходники которого называют установленное расширение `MyExt`, не
    /// даёт второго набора: пока раннер называет расширение по имени набора (#218), обход
    /// отказывает и просит переименовать набор.
    #[test]
    fn a_set_holding_an_installed_extension_under_another_name_is_not_doubled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = config_in(
            dir.path(),
            vec![
                set("main", SourceSetPurpose::Configuration, "src/cf"),
                set("my-ext", SourceSetPurpose::Extension, "src/ext"),
            ],
        );
        let root = config.base_path.join("src/ext");
        std::fs::create_dir_all(&root).expect("set dir");
        std::fs::write(
            root.join("Configuration.xml"),
            "<MetaDataObject><Configuration><Properties><Name>MyExt</Name><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>\n",
        )
        .expect("descriptor");

        let refusal = plan_walk(&config, &installed(&["MyExt"])).expect_err("doubled");

        let message = refusal.to_string();
        assert!(message.contains("'my-ext'"), "{message}");
        assert!(message.contains("rename the set to 'MyExt'"), "{message}");

        // Исходники, называющие то же расширение, что и имя набора, — обычный набор.
        let walk = plan_walk(&config, &installed(&["My-Ext"]));
        assert!(walk.is_ok(), "{walk:?}");
    }
}
