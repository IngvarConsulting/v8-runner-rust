//! `status`: состояние пары «каталог ↔ база».
//!
//! Без `--deep` ответ собран только из памяти под `workPath` — память о базе у каждого
//! набора, запись журнала поколений, признак нового владельца и файлы, изменившиеся с
//! последнего чтения каталога; ни одна утилита платформы не запускается
//! (`INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM`). `--all` отвечает так же о каждой базе,
//! объявленной в местном слое.
//!
//! `--deep` спрашивает платформу: поколение каждого набора тем инструментом, которым его
//! сверит `push`, и ту же сверку с записью (`INV.CLI.STATUS-DEEP-PREDICTS-THE-PUSH-GENERATION-CHECK`);
//! состав расширений базы рядом с наборами проекта
//! (`INV.CLI.STATUS-DEEP-NAMES-AN-EXTENSION-WITHOUT-A-PROJECT`); копии, которые держат файловую
//! базу (`INV.CLI.STATUS-DEEP-NAMES-THE-OWNING-COPY`). Ничего не пишет: ни памяти, ни метки.

use std::path::Path;
use std::time::Instant;

use crate::change_detection::analyzer::{self, AnalysisOutcome};
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::{AppConfig, SourceFormat};
use crate::domain::capability::{Operation, Provider};
use crate::domain::source_set::SourceSetContext;
use crate::domain::status::{
    BaseGeneration, ExtensionsStatus, GenerationVerdict, InfobaseStatus, InstalledExtensionStatus,
    MemoryState, RecordedGeneration, SourceSetStatus, StatusResult,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    connect, generation_id, transcript_log, wait_policy, AgentHandle, GenerationComparison,
    GenerationLedger, GenerationRecord, Recorded,
};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::exchange_guard::{memory_of, new_owner_mark};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};

/// Что спрашивают у `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusRequest {
    /// Спросить платформу.
    pub deep: bool,
    /// Ответить о каждой объявленной базе.
    pub all: bool,
}

/// Состояние выбранной базы или, у `--all`, каждой объявленной.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: StatusRequest,
) -> UseCaseResult<StatusResult> {
    let started = Instant::now();
    let infobases = if request.all {
        declared(config)
            .iter()
            .map(|base| from_memory(base, base.infobase_name == config.infobase_name))
            .collect()
    } else {
        let mut status = from_memory(config, true);
        if request.deep {
            deepen(context, config, &mut status).map_err(UseCaseFailure::without_payload)?;
        }
        vec![status]
    };
    Ok(StatusResult {
        deep: request.deep,
        infobases,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Каждая база местного слоя как выбранная: та же конфигурация с другой секцией базы.
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
    let service = SourceSetsService::new(config);
    let edt = service.edt_contexts();
    let source_sets = service
        .designer_contexts()
        .iter()
        .zip(&config.source_sets)
        .filter(|(_, set)| !set.purpose.is_external())
        .map(|(context, set)| {
            let memory = memory_of(context, &config.work_path);
            let sources = match config.format {
                SourceFormat::Designer => Some(context),
                SourceFormat::Edt => edt.iter().find(|edt| edt.name() == set.name),
            };
            SourceSetStatus {
                name: set.name.clone(),
                purpose: set.purpose.as_str().to_owned(),
                memory,
                recorded: own_record(context, &config.work_path).map(|record| RecordedGeneration {
                    token: record.token,
                    tool: record.tool,
                    // Имя операции — то же, что в журнале: его даёт сам тип записи.
                    after: serde_json::to_value(record.after)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    recorded_at: record.recorded_at,
                }),
                changed_files: (memory == MemoryState::Remembered)
                    .then(|| sources.and_then(|sources| changed_files(sources, &config.work_path)))
                    .flatten(),
                base: None,
            }
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
        new_owner_since: new_owner_mark(config),
        source_sets,
        extensions: None,
        holders: None,
    }
}

/// Запись журнала поколений этой пары «база ↔ каталог».
fn own_record(context: &SourceSetContext, work_path: &Path) -> Option<GenerationRecord> {
    match GenerationLedger::of(context, work_path)?.read() {
        Recorded::Ours(record) => Some(record),
        Recorded::Nothing | Recorded::Foreign { .. } => None,
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

/// То, что знает только платформа: поколение, состав расширений и метка владельца.
fn deepen(
    context: &ExecutionContext,
    config: &AppConfig,
    status: &mut InfobaseStatus,
) -> Result<(), AppError> {
    let contexts = SourceSetsService::new(config).designer_contexts();
    let mut reader = GenerationReader::open(context, config);
    for set in &mut status.source_sets {
        if let Some(error) = crate::use_cases::interruption::pending_interruption_error(
            context,
            "the configuration generation",
        ) {
            reader.close();
            return Err(error);
        }
        let Some(source) = contexts
            .iter()
            .find(|candidate| candidate.name() == set.name)
        else {
            continue;
        };
        let extension = config
            .source_sets
            .iter()
            .any(|declared| {
                declared.name == set.name
                    && declared.purpose == crate::config::model::SourceSetPurpose::Extension
            })
            .then_some(set.name.as_str());
        let (tool, answer) = reader.read(context, config, extension);
        let record = own_record(source, &config.work_path);
        set.base = Some(verdict(tool, answer, record.as_ref()));
    }
    reader.close();
    if let Some(error) = crate::use_cases::interruption::pending_interruption_error(
        context,
        "the extension inventory",
    ) {
        return Err(error);
    }
    status.extensions = Some(extensions(context, config));
    if let Some(error) =
        crate::use_cases::interruption::pending_interruption_error(context, "the owner marker")
    {
        return Err(error);
    }
    status.holders = crate::use_cases::infobase_owner::holders(config);
    Ok(())
}

/// Сверка ответа с записью — та же, что `push` делает перед загрузкой.
fn verdict(
    tool: Option<Provider>,
    answer: Result<Option<String>, String>,
    record: Option<&GenerationRecord>,
) -> BaseGeneration {
    let (token, reason) = match answer {
        Ok(token) => (token, None),
        Err(reason) => (None, Some(reason)),
    };
    let comparison = match (tool, token.as_deref(), record) {
        (_, _, None) => GenerationVerdict::NoRecord,
        (None, _, Some(_)) | (_, None, Some(_)) => GenerationVerdict::NoAnswer,
        (Some(tool), Some(token), Some(record)) => match record.compare(tool, token) {
            GenerationComparison::Unchanged => GenerationVerdict::Unchanged,
            GenerationComparison::Changed => GenerationVerdict::MovedAhead,
            GenerationComparison::NoAnswer => GenerationVerdict::OtherTool,
        },
    };
    let reason = reason.or_else(|| {
        (token.is_none() && tool.is_some())
            .then(|| "the tool gave no configuration generation".to_owned())
    });
    BaseGeneration {
        tool,
        token,
        comparison,
        reason,
    }
}

/// Состав расширений базы рядом с наборами расширений проекта. Имена сравниваются без
/// учёта регистра, как их сравнивает платформа.
fn extensions(context: &ExecutionContext, config: &AppConfig) -> ExtensionsStatus {
    let (provider, read) = crate::use_cases::extension_inventory::read_installed(context, config);
    let project: Vec<&str> = config
        .source_sets
        .iter()
        .filter(|set| set.purpose == crate::config::model::SourceSetPurpose::Extension)
        .map(|set| set.name.as_str())
        .collect();
    match read {
        Ok(installed) => {
            let same = |left: &str, right: &str| left.to_lowercase() == right.to_lowercase();
            let in_project = |name: &str| {
                project
                    .iter()
                    .find(|set| same(set, name))
                    .map(|set| (*set).to_owned())
            };
            let missing_in_base = project
                .iter()
                .filter(|set| !installed.iter().any(|extension| same(&extension.name, set)))
                .map(|set| (*set).to_owned())
                .collect();
            ExtensionsStatus {
                provider,
                installed: Some(
                    installed
                        .into_iter()
                        .map(|extension| InstalledExtensionStatus {
                            source_set: in_project(&extension.name),
                            active: extension.active,
                            name: extension.name,
                        })
                        .collect(),
                ),
                missing_in_base: Some(missing_in_base),
                reason: None,
            }
        }
        Err(error) => ExtensionsStatus {
            provider,
            installed: None,
            missing_in_base: None,
            reason: Some(error.to_string()),
        },
    }
}

/// Кто спрашивает поколение: исполнитель `push` для этой базы, одна сессия агента на команду.
enum GenerationReader {
    Designer {
        binary: std::path::PathBuf,
        utilities: PlatformUtilities,
    },
    Ibcmd {
        binary: std::path::PathBuf,
        connection: IbcmdConnection,
        utilities: PlatformUtilities,
    },
    Agent {
        handle: Option<AgentHandle>,
        wait: crate::platform::agent::WaitPolicy,
    },
    /// Спросить некем: почему, и кто был выбран, если был.
    Nobody {
        tool: Option<Provider>,
        reason: String,
    },
}

impl GenerationReader {
    fn open(context: &ExecutionContext, config: &AppConfig) -> Self {
        let mut utilities = PlatformUtilities::from_config(config);
        let selected = match crate::use_cases::provider_selection::select(
            config,
            &mut utilities,
            Operation::Build,
        ) {
            Ok(selected) => selected,
            Err((error, _)) => {
                return Self::Nobody {
                    tool: None,
                    reason: error.to_string(),
                }
            }
        };
        let tool = selected.provider;
        if !crate::platform::generation::answers_generation(tool, config.target_kind()) {
            return Self::Nobody {
                tool: Some(tool),
                reason: format!(
                    "{} gives no configuration generation for this infobase",
                    tool.as_str()
                ),
            };
        }
        match (tool, selected.location) {
            (Provider::Designer, Some(location)) => Self::Designer {
                binary: location.path,
                utilities,
            },
            (Provider::Ibcmd, Some(location)) => {
                match IbcmdConnection::from_infobase(&config.infobase) {
                    Ok(connection) => Self::Ibcmd {
                        binary: location.path,
                        connection,
                        utilities,
                    },
                    Err(error) => Self::Nobody {
                        tool: Some(tool),
                        reason: AppError::from(error).to_string(),
                    },
                }
            }
            (Provider::Agent, location) => {
                let wait = wait_policy(context);
                let handle = transcript_log(config, "status").and_then(|log| {
                    connect(
                        context,
                        config,
                        &mut utilities,
                        location.as_ref().map(|location| location.path.as_path()),
                        log,
                        &wait,
                    )
                });
                match handle {
                    Ok(handle) => Self::Agent {
                        handle: Some(handle),
                        wait,
                    },
                    Err(error) => Self::Nobody {
                        tool: Some(tool),
                        reason: error.to_string(),
                    },
                }
            }
            (other, _) => Self::Nobody {
                tool: Some(other),
                reason: format!(
                    "{} has no adapter that reads the configuration generation",
                    other.as_str()
                ),
            },
        }
    }

    /// Поколение основной конфигурации или расширения `extension`: инструмент и ответ;
    /// ошибка — почему ответа нет.
    fn read(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        extension: Option<&str>,
    ) -> (Option<Provider>, Result<Option<String>, String>) {
        let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
        match self {
            Self::Designer { binary, utilities } => {
                let log = crate::support::temp::platform_logs_dir(&config.work_path)
                    .map(|dir| dir.join("status-generation.log"));
                let answer = match log {
                    Ok(log) => DesignerDsl::new(
                        binary.clone(),
                        config.v8_connection(),
                        utilities.runner_for(UtilityType::V8),
                        Some(log),
                        policy,
                    )
                    .config_generation_id(extension)
                    .map_err(|error| AppError::from(error).to_string()),
                    Err(error) => Err(format!("failed to create platform logs dir: {error}")),
                };
                (Some(Provider::Designer), answer)
            }
            Self::Ibcmd {
                binary,
                connection,
                utilities,
            } => (
                Some(Provider::Ibcmd),
                IbcmdDsl::new(
                    binary.clone(),
                    connection.clone(),
                    utilities.runner_for(UtilityType::Ibcmd),
                    policy,
                )
                .config_generation_id(extension)
                .map_err(|error| AppError::from(error).to_string()),
            ),
            Self::Agent { handle, wait } => (
                Some(Provider::Agent),
                match handle.as_mut() {
                    Some(handle) => generation_id(handle.session(), extension, wait)
                        .map(Some)
                        .map_err(|error| error.to_string()),
                    None => Err("the agent session is closed".to_owned()),
                },
            ),
            Self::Nobody { tool, reason } => (*tool, Err(reason.clone())),
        }
    }

    fn close(&mut self) {
        if let Self::Agent { handle, wait } = self {
            if let Some(handle) = handle.take() {
                handle.finish(wait);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tool: Provider, token: &str) -> GenerationRecord {
        GenerationRecord {
            token: token.to_owned(),
            tool,
            after: crate::use_cases::agent_session::GenerationAfter::Build,
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
        let cases = [
            (
                Some(Provider::Designer),
                Ok(Some(a.clone())),
                Some(&designer),
                GenerationVerdict::Unchanged,
            ),
            (
                Some(Provider::Designer),
                Ok(Some(b.clone())),
                Some(&designer),
                GenerationVerdict::MovedAhead,
            ),
            (
                Some(Provider::Ibcmd),
                Ok(Some(b.clone())),
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
                Err("no executor".to_owned()),
                Some(&designer),
                GenerationVerdict::NoAnswer,
            ),
            (
                Some(Provider::Designer),
                Ok(Some(a.clone())),
                None,
                GenerationVerdict::NoRecord,
            ),
        ];
        for (tool, answer, record, expected) in cases {
            assert_eq!(verdict(tool, answer, record).comparison, expected);
        }
        assert_eq!(
            verdict(Some(Provider::Designer), Ok(None), Some(&designer))
                .reason
                .as_deref(),
            Some("the tool gave no configuration generation")
        );
    }
}
