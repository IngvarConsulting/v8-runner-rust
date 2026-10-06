//! `status`: состояние пары «каталог ↔ база».
//!
//! Без `--deep` ответ собран только из памяти под `workPath` — память о базе у каждого
//! набора, запись журнала поколений, признак нового владельца и файлы, изменившиеся с
//! последнего чтения каталога; ни одна утилита платформы не запускается
//! (`INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM`). `--all` отвечает так же о каждой базе,
//! объявленной в местном слое.
//!
//! `--deep` спрашивает платформу: поколение каждого набора тем исполнителем, которым его
//! сверит `push`, и та же сверка с записью (`exchange_guard::predict`,
//! `INV.CLI.STATUS-DEEP-PREDICTS-THE-PUSH-GENERATION-CHECK`); состав расширений базы рядом с
//! наборами проекта (`INV.CLI.STATUS-DEEP-NAMES-AN-EXTENSION-WITHOUT-A-PROJECT`); копии,
//! которые держат файловую базу (`INV.CLI.STATUS-DEEP-NAMES-THE-OWNING-COPY`). Ни памяти, ни
//! метки он не пишет — только журналы платформы и сессии агента.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::change_detection::analyzer::{self, AnalysisOutcome};
use crate::config::model::{AppConfig, SourceFormat};
use crate::domain::capability::{Operation, Provider};
use crate::domain::source_set::SourceSetContext;
use crate::domain::status::{
    BaseGeneration, ExtensionsStatus, InfobaseStatus, InstalledExtensionStatus, MemoryState,
    RecordedGeneration, SourceSetStatus, StatusResult, StatusScope,
};
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    connect, generation_id, transcript_log, wait_policy, AgentHandle, GenerationRecord,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::exchange_guard::{memory_of, new_owner_since, predict, recorded_generation};
use crate::use_cases::generation_reader::{designer_log_file, read_generation, GenerationProcess};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;

/// Состояние выбранной базы или, у `--all`, каждой объявленной.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    scope: StatusScope,
) -> UseCaseResult<StatusResult> {
    let started = Instant::now();
    let infobases = match scope {
        StatusScope::All => declared(config)
            .iter()
            .map(|base| from_memory(base, base.infobase_name == config.infobase_name))
            .collect(),
        StatusScope::Selected => vec![from_memory(config, true)],
        StatusScope::Deep => {
            let mut status = from_memory(config, true);
            deepen(context, config, &mut status).map_err(UseCaseFailure::without_payload)?;
            vec![status]
        }
    };
    Ok(StatusResult {
        deep: scope == StatusScope::Deep,
        infobases,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Каждая база местного слоя как выбранная: та же конфигурация с другой секцией базы. База,
/// пришедшая строкой соединения в `--infobase`, в местном слое не объявлена и в перечень не
/// входит.
fn declared(config: &AppConfig) -> Vec<AppConfig> {
    config
        .infobases
        .iter()
        .map(|(name, infobase)| AppConfig {
            infobase: infobase.clone(),
            infobase_name: Some(name.clone()),
            ..config.clone()
        })
        .collect()
}

/// Ответ по памяти под `workPath`: платформа не запускается.
fn from_memory(config: &AppConfig, selected: bool) -> InfobaseStatus {
    let base_path = crate::support::path::absolute_from_current_dir(&config.base_path)
        .unwrap_or_else(|_| config.base_path.clone());
    let inventory = SourceSetInventory::new(config);
    let source_sets = inventory
        .configuration_packages()
        .into_iter()
        .filter_map(|(set, _)| {
            let context = inventory.designer_context(&set.name)?;
            let memory = memory_of(context, &config.work_path);
            let sources = match config.format {
                SourceFormat::Designer => Some(context),
                SourceFormat::Edt => inventory.edt_context(&set.name),
            };
            Some(SourceSetStatus {
                name: set.name.clone(),
                purpose: set.purpose,
                memory,
                recorded: recorded_generation(context, &config.work_path).map(|record| {
                    RecordedGeneration {
                        token: record.token,
                        tool: record.tool,
                        after: record.after,
                        recorded_at: record.recorded_at,
                    }
                }),
                changed_files: sources
                    .filter(|_| memory == MemoryState::Remembered)
                    .and_then(|sources| changed_files(sources, &config.work_path)),
                base: None,
            })
        })
        .collect();
    InfobaseStatus {
        name: config.infobase_name.clone(),
        selected,
        kind: config.target_kind(),
        // Память привязана к адресу с хешем пути; человеку показывается сам путь.
        address: match config.v8_connection().file_infobase_dir(&base_path) {
            Some(dir) => Some(format!("file:{}", dir.display())),
            None => config.infobase_memory_address(&base_path),
        },
        new_owner_since: new_owner_since(config),
        source_sets,
        extensions: None,
        holders: None,
    }
}

/// Сколько файлов каталога изменилось с последнего чтения; анализ ничего не записывает.
/// `None` — сравнить не с чем: памяти нет или хранилище не читается.
fn changed_files(context: &SourceSetContext, work_path: &Path) -> Option<u64> {
    match analyzer::analyze_context(context, work_path).outcome {
        Ok(AnalysisOutcome::NoChanges) => Some(0),
        Ok(AnalysisOutcome::Changes { changes, .. }) => Some(changes.len() as u64),
        Ok(AnalysisOutcome::Fallback) | Err(_) => None,
    }
}

/// Отмена между вопросами к платформе: безопасная точка, после которой вопросов нет.
fn interrupted(context: &ExecutionContext, place: &str) -> Result<(), AppError> {
    match crate::use_cases::interruption::pending_interruption_error(context, place) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// То, что знает только платформа: поколение, состав расширений и метка владельца.
fn deepen(
    context: &ExecutionContext,
    config: &AppConfig,
    status: &mut InfobaseStatus,
) -> Result<(), AppError> {
    let inventory = SourceSetInventory::new(config);
    let packages = inventory.configuration_packages();
    let asker = Asker::open(context, config);
    let (tool, mut asker) = match asker {
        Ok(asker) => (Some(asker.tool), Ok(asker)),
        Err((tool, error)) => (tool, Err(error)),
    };
    let mut outcome = Ok(());
    for set in &mut status.source_sets {
        if let Err(error) = interrupted(context, "the configuration generation") {
            outcome = Err(error);
            break;
        }
        let Some((declared, extension)) = packages
            .iter()
            .find(|(declared, _)| declared.name == set.name)
        else {
            continue;
        };
        let record = inventory
            .designer_context(&declared.name)
            .and_then(|context| recorded_generation(context, &config.work_path));
        set.base = Some(match asker.as_mut() {
            Ok(asker) => base_generation(
                tool,
                asker
                    .read(context, config, *extension)
                    .as_ref()
                    .map(Option::as_deref),
                record.as_ref(),
            ),
            Err(error) => base_generation(tool, Err(&*error), record.as_ref()),
        });
    }
    if let Ok(asker) = asker {
        asker.close();
    }
    outcome?;
    interrupted(context, "the extension inventory")?;
    status.extensions = Some(extensions(context, config, &packages));
    interrupted(context, "the owner marker")?;
    status.holders = crate::use_cases::infobase_owner::holders(config);
    Ok(())
}

/// Ответ о поколении набора и та же сверка с записью, что сделает `push`.
fn base_generation(
    tool: Option<Provider>,
    answer: Result<Option<&str>, &AppError>,
    record: Option<&GenerationRecord>,
) -> BaseGeneration {
    let (token, reason) = match answer {
        Ok(Some(token)) => (Some(token.to_owned()), None),
        Ok(None) => (
            None,
            Some("the tool gave no configuration generation".to_owned()),
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    let comparison = predict(record, tool.zip(token.as_deref()));
    BaseGeneration {
        tool,
        token,
        comparison,
        reason,
    }
}

/// Состав расширений базы рядом с наборами расширений проекта. Имена 1С регистр не
/// различают: сопоставление без регистра, как у `pull --all`.
fn extensions(
    context: &ExecutionContext,
    config: &AppConfig,
    packages: &[(&crate::config::model::SourceSetConfig, Option<&str>)],
) -> ExtensionsStatus {
    let (provider, read) = crate::use_cases::extension_inventory::read_installed(context, config);
    let installed = match read {
        Ok(installed) => installed,
        Err(error) => {
            return ExtensionsStatus {
                provider,
                installed: None,
                missing_in_base: None,
                reason: Some(error.to_string()),
            }
        }
    };
    let project: Vec<(String, &str)> = packages
        .iter()
        .filter_map(|(set, extension)| {
            extension.map(|name| (name.to_lowercase(), set.name.as_str()))
        })
        .collect();
    let tool = config
        .tools
        .client_mcp
        .extension
        .as_ref()
        .map(|tool| tool.name.to_lowercase());
    let installed: Vec<(String, _)> = installed
        .into_iter()
        .map(|extension| (extension.name.to_lowercase(), extension))
        .collect();
    let missing_in_base = project
        .iter()
        .filter(|(key, _)| !installed.iter().any(|(installed, _)| installed == key))
        .map(|(_, set)| (*set).to_owned())
        .collect();
    ExtensionsStatus {
        provider,
        installed: Some(
            installed
                .into_iter()
                .map(|(key, extension)| InstalledExtensionStatus {
                    source_set: project
                        .iter()
                        .find(|(project, _)| *project == key)
                        .map(|(_, set)| (*set).to_owned()),
                    tool: tool.as_deref() == Some(key.as_str()),
                    active: extension.active,
                    name: extension.name,
                })
                .collect(),
        ),
        missing_in_base: Some(missing_in_base),
        reason: None,
    }
}

/// Кто спрашивает поколение: исполнитель `push` для этой базы, одна сессия агента на
/// команду. Процесс платформы зовёт общий читатель (`generation_reader`).
struct Asker {
    tool: Provider,
    how: How,
}

enum How {
    Designer {
        binary: PathBuf,
        utilities: PlatformUtilities,
    },
    Ibcmd {
        binary: PathBuf,
        utilities: PlatformUtilities,
    },
    Agent {
        /// Сессия крупнее процесса утилиты, поэтому лежит в куче.
        handle: Box<AgentHandle>,
        wait: WaitPolicy,
    },
}

impl Asker {
    /// Исполнитель `push`, готовый спросить поколение. Отказ — исполнитель и почему ответа не
    /// будет: исполнителя нет, `push` этим исполнителем такой проект не грузит или он
    /// поколением у этой базы не отвечает.
    fn open(
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<Self, (Option<Provider>, AppError)> {
        if let Some(error) = crate::use_cases::build_project::unsupported_push_executor(config) {
            return Err((Some(config.selected_provider(Operation::Build)), error));
        }
        let mut utilities = PlatformUtilities::from_config(config);
        let selected =
            crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Build)
                .map_err(|(error, _)| (None, error))?;
        let tool = selected.provider;
        if !crate::platform::generation::answers_generation(tool, config.target_kind()) {
            return Err((
                Some(tool),
                AppError::capability(format!(
                    "{tool} gives no configuration generation for this infobase"
                )),
            ));
        }
        let how = match (tool, selected.location) {
            (Provider::Designer, Some(location)) => How::Designer {
                binary: location.path,
                utilities,
            },
            (Provider::Ibcmd, Some(location)) => How::Ibcmd {
                binary: location.path,
                utilities,
            },
            (Provider::Agent, location) => {
                let wait = wait_policy(context);
                let handle = transcript_log(config, "status")
                    .and_then(|log| {
                        connect(
                            context,
                            config,
                            &mut utilities,
                            location.as_ref().map(|location| location.path.as_path()),
                            log,
                            &wait,
                        )
                    })
                    .map_err(|error| (Some(tool), error))?;
                How::Agent {
                    handle: Box::new(handle),
                    wait,
                }
            }
            (other, _) => {
                return Err((
                    Some(other),
                    crate::use_cases::unimplemented_provider(Operation::Build, other),
                ))
            }
        };
        Ok(Self { tool, how })
    }

    /// Поколение основной конфигурации или расширения `extension`; `None` — ответа нет.
    fn read(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        extension: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        match &mut self.how {
            How::Designer { binary, utilities } => read_generation(
                context,
                config,
                GenerationProcess::Designer {
                    binary,
                    runner: utilities.runner_for(UtilityType::V8),
                    log_file: designer_log_file(config, "status-generation")?,
                },
                extension,
            ),
            How::Ibcmd { binary, utilities } => read_generation(
                context,
                config,
                GenerationProcess::Ibcmd {
                    binary,
                    runner: utilities.runner_for(UtilityType::Ibcmd),
                    data_path: None,
                },
                extension,
            ),
            How::Agent { handle, wait } => {
                generation_id(handle.session(), extension, wait).map(Some)
            }
        }
    }

    fn close(self) {
        if let How::Agent { handle, wait } = self.how {
            handle.finish(&wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::status::{GenerationAfter, GenerationVerdict};

    fn record(tool: Provider, token: &str) -> GenerationRecord {
        GenerationRecord {
            token: token.to_owned(),
            tool,
            after: GenerationAfter::Build,
            recorded_at: "2026-10-06T00:00:00Z".to_owned(),
            identity: "pair".to_owned(),
        }
    }

    /// Сверка ответа с записью идёт только внутри одного инструмента, как у `push`; ответа
    /// нет — нет и сверки, записи нет — сравнивать не с чем.
    #[test]
    fn the_verdict_compares_within_the_tool_of_the_record() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let designer = record(Provider::Designer, &a);
        let failure = AppError::capability("no executor".to_owned());
        let cases = [
            (
                Some(Provider::Designer),
                Ok(Some(a.as_str())),
                Some(&designer),
                GenerationVerdict::Unchanged,
            ),
            (
                Some(Provider::Designer),
                Ok(Some(b.as_str())),
                Some(&designer),
                GenerationVerdict::MovedAhead,
            ),
            (
                Some(Provider::Ibcmd),
                Ok(Some(b.as_str())),
                Some(&designer),
                GenerationVerdict::OtherTool,
            ),
            (
                Some(Provider::Designer),
                Ok(None),
                Some(&designer),
                GenerationVerdict::NoAnswer,
            ),
            (
                None,
                Err(&failure),
                Some(&designer),
                GenerationVerdict::NoAnswer,
            ),
            (
                Some(Provider::Designer),
                Ok(Some(a.as_str())),
                None,
                GenerationVerdict::NoRecord,
            ),
        ];
        for (tool, answer, record, expected) in cases {
            assert_eq!(base_generation(tool, answer, record).comparison, expected);
        }
        assert_eq!(
            base_generation(Some(Provider::Designer), Ok(None), Some(&designer))
                .reason
                .as_deref(),
            Some("the tool gave no configuration generation")
        );
    }
}
