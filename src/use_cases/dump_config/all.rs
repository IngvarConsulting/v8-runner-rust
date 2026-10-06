//! `pull --all`: наборы по составу базы.
//!
//! Команда спрашивает базу, какие расширения в ней установлены (замер #187: у каждого
//! исполнителя свой вызов, ответ разбирается по структуре, а не по тексту сообщений), и
//! обходит пакеты конфигурации одним порядком [`SourceSetInventory::configuration_packages`]:
//! сперва наборы проекта, затем по набору `src/ext/<Name>` на каждое расширение без набора.
//! Каждый набор выгружается тем же сценарием, что `pull <SET>`, со сторожем каталога и
//! памятью базы; объявление нового набора дописывается в `v8project.yaml` после его удачной
//! выгрузки, так что отказ посреди обхода не оставляет в проекте набора без содержимого.

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
use crate::use_cases::provider_selection::SelectedProvider;
use crate::use_cases::request::PullAllRequest;
use crate::use_cases::result::UseCaseError;

/// Каталог, под которым `pull --all` заводит набор расширения: `src/ext/<Name>` от каталога
/// проектного файла.
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
        sets: Vec::new(),
        duration_ms: 0,
        message: None,
    };
    let refuse = |error: AppError, mut result: PullAllResult| {
        result.duration_ms = started.elapsed().as_millis() as u64;
        result.message = Some(error.to_string());
        PullAllFailure::with_payload(error, result)
    };

    if let Some(error) = validate_supported_matrix(config) {
        return Err(refuse(error, result));
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let selected =
        match crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Dump)
        {
            Ok(selected) => selected,
            Err((error, receipt)) => {
                result.provider = Some(receipt);
                return Err(refuse(error, result));
            }
        };
    result.provider = Some(selected.receipt.clone());

    if request.dry_run {
        // Состав базы читает платформа, а превью её не запускает: объявлять пока нечего, и
        // `declared` остаётся `null`. Наборы проекта названы превью их выгрузки.
        for (source_set, _) in SourceSetInventory::new(config).configuration_packages() {
            pull_one(
                context,
                config,
                request,
                &source_set.name,
                SetKind::Project,
                &mut result,
            )
            .map_err(|error| finish_failure(error, result.clone(), started))?;
        }
        result.message = Some(format!(
            "would read the extensions installed in the infobase via {} and, for each one without a source-set, declare `{DECLARED_EXTENSION_ROOT}/<Name>` in '{}' and pull it there; nothing read, nothing written",
            provider_label(&selected),
            request.project_file.display()
        ));
        result.ok = true;
        result.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(result);
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
    let walk = match plan_walk(config, &installed).and_then(|walk| {
        // Дописать проектный файл должно получиться до первой выгрузки: иначе выгруженное
        // осталось бы без объявления.
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
        Err(error) => return Err(refuse(error, result)),
    };
    result.not_installed = walk.not_installed.clone();
    result.declared = Some(Vec::new());

    // Наборы проекта и объявляемые идут одним обходом: в настройках этой команды новые
    // наборы уже есть, на диске — появляются после своей выгрузки.
    let mut walked = config.clone();
    walked
        .source_sets
        .extend(walk.declared.iter().map(|declared| SourceSetConfig {
            name: declared.name.clone(),
            purpose: SourceSetPurpose::Extension,
            path: PathBuf::from(&declared.path),
        }));
    for name in &walk.existing {
        pull_one(
            context,
            &walked,
            request,
            name,
            SetKind::Project,
            &mut result,
        )
        .map_err(|error| finish_failure(error, result.clone(), started))?;
    }
    for declared in &walk.declared {
        pull_one(
            context,
            &walked,
            request,
            &declared.name,
            SetKind::Declared,
            &mut result,
        )
        .map_err(|error| finish_failure(error, result.clone(), started))?;
        declare_source_sets(&request.project_file, std::slice::from_ref(declared))
            .map_err(|error| finish_failure(error.into(), result.clone(), started))?;
        result
            .declared
            .get_or_insert_with(Vec::new)
            .push(declared.clone());
    }
    result.ok = true;
    result.duration_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}

/// Чем выгружается набор обхода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetKind {
    /// Набор проекта: как `pull <SET>` — по изменившемуся, с `--force` полной заменой.
    Project,
    /// Объявляемый набор: каталога у него ещё нет, и выгрузка полная, как первая. Сторож
    /// спрашивается так же — каталог, оставшийся от прерванного прогона, без согласия не
    /// заменяется, — а полная выгрузка записывает хеши и копию файла версий.
    Declared,
}

/// Выгружает один набор сценарием `pull <SET>` и кладёт его ответ в обход; отказ набора
/// останавливает обход.
fn pull_one(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &PullAllRequest,
    source_set: &str,
    kind: SetKind,
    result: &mut PullAllResult,
) -> Result<(), UseCaseError> {
    let set_request = DumpArgs {
        mode: if request.discard_uncommitted || kind == SetKind::Declared {
            DumpModeRequest::Full
        } else {
            DumpModeRequest::Incremental
        },
        source_set: Some(source_set.to_owned()),
        extension: None,
        objects: Vec::new(),
        dry_run: request.dry_run,
        discard_uncommitted: request.discard_uncommitted,
        force_way_out: ForceWayOut::PullForce,
    };
    match super::execute(context, config, &set_request) {
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

/// Отказ посреди обхода: ответ несёт выгруженное до него и называет причину.
fn finish_failure(
    error: UseCaseError,
    mut result: PullAllResult,
    started: Instant,
) -> PullAllFailure {
    result.duration_ms = started.elapsed().as_millis() as u64;
    result.message = Some(error.to_string());
    PullAllFailure::with_payload(error, result)
}

/// Что обойти: наборы проекта в порядке пакетов и наборы, которые надо объявить.
#[derive(Debug, PartialEq, Eq)]
struct Walk {
    /// Пакеты проекта в порядке обхода: основная конфигурация и расширения, которые в базе
    /// есть.
    existing: Vec<String>,
    /// Наборы для расширений базы без набора, по имени.
    declared: Vec<ConfigInitSourceSet>,
    /// Наборы расширений проекта, которых в базе нет.
    not_installed: Vec<String>,
}

/// Сопоставляет состав базы с наборами проекта. Имена 1С регистр не различают, поэтому и
/// сопоставление без регистра: `old` в проекте — то же расширение, что `Old` в базе.
fn plan_walk(config: &AppConfig, installed: &[String]) -> Result<Walk, AppError> {
    let key = |name: &str| name.to_lowercase();
    let installed_keys = installed
        .iter()
        .map(|name| key(name))
        .collect::<std::collections::HashSet<_>>();
    let inventory = SourceSetInventory::new(config);
    let mut existing = Vec::new();
    let mut not_installed = Vec::new();
    for (source_set, extension) in inventory.configuration_packages() {
        match extension {
            Some(extension) if !installed_keys.contains(&key(extension)) => {
                not_installed.push(source_set.name.clone())
            }
            _ => existing.push(source_set.name.clone()),
        }
    }

    let mut declared = Vec::new();
    for name in installed {
        if let Some(taken) = config
            .source_sets
            .iter()
            .find(|source_set| key(&source_set.name) == key(name))
        {
            if taken.purpose == SourceSetPurpose::Extension {
                continue;
            }
            return Err(AppError::Validation(format!(
                "the infobase has extension '{name}', and its name is taken by the {} source-set '{}' in the project: an extension source-set cannot be declared under it; rename that source-set",
                taken.purpose.as_str(),
                taken.name
            )));
        }
        let path = format!("{DECLARED_EXTENSION_ROOT}/{name}");
        let root = config.base_path.join(&path);
        if let Some(taken) = config
            .source_sets
            .iter()
            .find(|source_set| source_set.root_in(&config.base_path) == root)
        {
            return Err(AppError::Validation(format!(
                "extension '{name}' would be declared at '{path}', which source-set '{}' already uses",
                taken.name
            )));
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
    if let Some(name) = names
        .iter()
        .find(|name: &&String| !is_extension_identifier(name))
    {
        return Err(AppError::InvalidOutput(format!(
            "the infobase lists an extension whose name is not an identifier: {name:?}"
        )));
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::{plan_walk, Walk};
    use crate::config::model::{
        AppConfig, InfobaseConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
    };
    use crate::domain::config_init::ConfigInitSourceSet;

    fn set(name: &str, purpose: SourceSetPurpose, path: &str) -> SourceSetConfig {
        SourceSetConfig {
            name: name.to_owned(),
            purpose,
            path: path.into(),
        }
    }

    fn config(source_sets: Vec<SourceSetConfig>) -> AppConfig {
        let root = std::env::temp_dir().join("v8-runner-pull-all-plan");
        AppConfig {
            base_path: root.join("base"),
            work_path: root.join("work"),
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

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
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

        let declared = |name: &str| ConfigInitSourceSet {
            name: name.to_owned(),
            source_type: "EXTENSION".to_owned(),
            path: format!("src/ext/{name}"),
        };
        assert_eq!(
            walk,
            Walk {
                existing: vec!["main".to_owned(), "old".to_owned()],
                declared: vec![declared("Second"), declared("Новое")],
                not_installed: vec!["gone".to_owned()],
            }
        );
    }

    /// Имя расширения, занятое набором другого назначения, и каталог, занятый другим
    /// набором, — отказ до выгрузки.
    #[test]
    fn a_taken_name_or_directory_is_refused() {
        let config = config(vec![
            set("main", SourceSetPurpose::Configuration, "src/cf"),
            set("other", SourceSetPurpose::Extension, "src/ext/Ext"),
        ]);

        let taken_name = plan_walk(&config, &installed(&["MAIN"])).expect_err("name");
        assert!(taken_name.to_string().contains("'main'"), "{taken_name}");
        let taken_path = plan_walk(&config, &installed(&["Ext"])).expect_err("path");
        assert!(taken_path.to_string().contains("'other'"), "{taken_path}");
    }
}
