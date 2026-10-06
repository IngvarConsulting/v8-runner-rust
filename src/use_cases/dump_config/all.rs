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

use super::helpers::ensure_success_of;
use super::*;
use crate::config::model::SourceSetConfig;
use crate::domain::capability::{Operation, Provider};
use crate::domain::config_init::ConfigInitSourceSet;
use crate::domain::dump::PullAllResult;
use crate::platform::extension_inventory::{
    is_extension_identifier, parse_extension_inventory, parse_extension_name_list,
};
use crate::use_cases::config_init::{declare_source_sets, with_declared_source_sets};
use crate::use_cases::extension_agent::ExtensionAgent;
use crate::use_cases::extension_identity::source_extension_name;
use crate::use_cases::provider_selection::SelectedProvider;
use crate::use_cases::request::PullAllRequest;
use crate::use_cases::result::UseCaseError;

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

type PullAllFailure = UseCaseFailure<PullAllResult>;

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
        if_installed: Vec::new(),
        sets: Vec::new(),
        duration_ms: 0,
        message: None,
    };

    if let Some(error) = validate_supported_matrix(config) {
        return Err(fail(error, result, started));
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let selected =
        match crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Dump)
        {
            Ok(selected) => selected,
            Err((error, receipt)) => {
                result.provider = Some(receipt);
                return Err(fail(error, result, started));
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

    let installed = match read_installed_extensions(context, config, &selected, &utilities) {
        Ok(installed) => installed,
        Err(error) => {
            return Err(UseCaseFailure::after_possible_work(
                error,
                context.work(),
                || {
                    let mut result = result.clone();
                    result.duration_ms = started.elapsed().as_millis() as u64;
                    result
                },
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
        Err(error) => return Err(fail(error, result, started)),
    };
    result.not_installed = walk
        .not_installed
        .iter()
        .map(|name| (*name).to_owned())
        .collect();

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
            return Err(fail(error, result, started));
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
            return Err(fail(error, result, started));
        }
        declared.push(set.clone());
    }
    result.declared = Some(declared);
    result.ok = true;
    result.duration_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}

/// Отказ: ответ несёт выгруженное до него и называет причину.
fn fail(
    error: impl Into<UseCaseError>,
    mut result: PullAllResult,
    started: Instant,
) -> PullAllFailure {
    let error = error.into();
    result.duration_ms = started.elapsed().as_millis() as u64;
    result.message = Some(error.to_string());
    PullAllFailure::with_payload(error, result)
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
                return Err(fail(error, result, started));
            }
        }
        result.message = Some(format!(
            "would read the extensions installed in the infobase via {}, pull each extension set of the project the infobase has and name the others as not installed, and, for each extension without a set, pull it into `{DECLARED_EXTENSION_ROOT}/<Name>` and then declare that set in '{}'; which sets would be declared is known only once the infobase is read; nothing read, nothing written",
            provider_label(selected),
            self.request.project_file.display()
        ));
        result.ok = true;
        result.duration_ms = started.elapsed().as_millis() as u64;
        Ok(result)
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
        match super::execute(self.context, self.config, &set_request) {
            Ok(pulled) => {
                result.sets.push(pulled);
                Ok(())
            }
            Err(failure) => {
                result.sets.extend(failure.payload);
                Err(failure.error)
            }
        }
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

/// Сопоставляет состав базы с наборами проекта по имени расширения — тому, каким набор
/// называет расширение платформе ([`SourceSetInventory::configuration_packages`]). Имена
/// 1С регистр не различают, поэтому и сопоставление без регистра: `old` в проекте — то же
/// расширение, что `Old` в базе.
///
/// Набор, исходники которого называют другое установленное расширение, — отказ: под
/// своим именем он выгрузил бы не то расширение, а объявить второй набор для того же
/// расширения значило бы раздвоить его. Расширение-инструмент клиентского MCP
/// (`tools.client_mcp.extension`) раннер ставит сам, и набором оно не объявляется.
fn plan_walk<'a>(config: &'a AppConfig, installed: &[String]) -> Result<Walk<'a>, AppError> {
    let key = |name: &str| name.to_lowercase();
    let installed_keys = installed
        .iter()
        .map(|name| key(name))
        .collect::<HashSet<_>>();
    let mut existing = Vec::new();
    let mut not_installed = Vec::new();
    let mut claimed = HashSet::new();
    for (source_set, extension) in SourceSetInventory::new(config).configuration_packages() {
        let Some(extension) = extension else {
            existing.push(source_set.name.as_str());
            continue;
        };
        let root = source_set.root_in(&config.base_path);
        if let Some(held) = source_extension_name(config.format, &root)?
            .filter(|held| key(held) != key(extension) && installed_keys.contains(&key(held)))
        {
            return Err(AppError::Validation(format!(
                "source-set '{}' holds extension '{held}' by the Name in its sources, but the runner pulls an extension set by the set's name, '{extension}' (#218): rename the set to '{held}' in the project file; no second set is declared for '{held}'",
                source_set.name
            )));
        }
        claimed.insert(key(extension));
        if installed_keys.contains(&key(extension)) {
            existing.push(source_set.name.as_str());
        } else {
            not_installed.push(source_set.name.as_str());
        }
    }

    let tool = config
        .tools
        .client_mcp
        .extension
        .as_ref()
        .map(|tool| key(&tool.name));
    let mut declared = Vec::new();
    for name in installed {
        if claimed.contains(&key(name)) || tool.as_deref() == Some(key(name).as_str()) {
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
        declared.push(ConfigInitSourceSet {
            name: name.clone(),
            source_type: SourceSetPurpose::Extension.as_str().to_owned(),
            path: format!("{DECLARED_EXTENSION_ROOT}/{name}"),
        });
    }
    declared.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Walk {
        existing,
        declared,
        not_installed,
    })
}

fn provider_label(selected: &SelectedProvider) -> String {
    match &selected.location {
        Some(found) => found.path.display().to_string(),
        None => "the attached Designer agent".to_owned(),
    }
}

/// Имена расширений, установленных в базе, — вызовом выбранного исполнителя (замер #187).
///
/// Имя, которое не является идентификатором 1С, — неверный вывод: оно станет каталогом и
/// доводом платформы, и угадывать его нельзя.
fn read_installed_extensions(
    context: &ExecutionContext,
    config: &AppConfig,
    selected: &SelectedProvider,
    utilities: &PlatformUtilities,
) -> Result<Vec<String>, AppError> {
    log_live_stage(
        "pull: extensions",
        "[Pull] reading the extensions installed in the infobase",
    );
    let binary = selected.location.as_ref().map(|found| found.path.as_path());
    let names = match (selected.provider, binary) {
        (Provider::Designer, Some(binary)) => {
            let dsl = build_designer_dsl(
                context,
                config,
                binary,
                utilities.runner_for(UtilityType::V8),
                "extensions",
                "list",
            )?;
            let listed = dsl
                .dump_db_cfg_list_all_extensions()
                .map_err(AppError::from)?;
            ensure_success_of(
                "list extensions of",
                "infobase",
                "the configured infobase",
                &listed,
            )?;
            let Some(out) = listed.platform_log.as_deref() else {
                return Err(AppError::InvalidOutput(format!(
                    "the Designer extension list was not read: {}",
                    listed
                        .platform_log_read_error
                        .as_deref()
                        .unwrap_or("no /Out log")
                )));
            };
            parse_extension_name_list(out).map_err(AppError::InvalidOutput)?
        }
        (Provider::Ibcmd, Some(binary)) => {
            let dsl = build_ibcmd_dsl(
                context,
                config,
                binary,
                utilities.runner_for(UtilityType::Ibcmd),
            )?;
            let listed = dsl.infobase_extension_list().map_err(map_ibcmd_error)?;
            ensure_success_of(
                "list extensions of",
                "infobase",
                "the configured infobase",
                &listed,
            )?;
            parse_extension_inventory(&listed.process.stdout)
                .map_err(AppError::InvalidOutput)?
                .into_iter()
                .map(|extension| extension.name)
                .collect()
        }
        (Provider::Agent, binary) => {
            let mut agent = ExtensionAgent::open(context, config, binary)?;
            let inventory = agent.inventory(None);
            agent.close();
            inventory?
                .into_iter()
                .map(|extension| extension.name)
                .collect()
        }
        // Без утилиты исполнитель списка не прочтёт, а прочие исполнители выгрузку не
        // делают: тот же отказ, что у выгрузки без адаптера.
        (provider @ (Provider::Designer | Provider::Ibcmd), None)
        | (provider @ (Provider::IbcmdRs | Provider::Webinst), _) => {
            return Err(crate::use_cases::unimplemented_provider(
                Operation::Dump,
                provider,
            ))
        }
    };
    if let Some(name) = names.iter().find(|name| !is_extension_identifier(name)) {
        return Err(AppError::InvalidOutput(format!(
            "the infobase lists an extension whose name is not an identifier usable as a directory name: {name:?}"
        )));
    }
    Ok(names)
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
            }
        );
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
