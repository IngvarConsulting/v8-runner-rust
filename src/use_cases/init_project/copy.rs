//! `infobase create --from <база>`: база этой рабочей копии — копия другой объявленной базы с
//! её данными и конфигурацией (`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).
//!
//! С источника снимается образ DT — Конфигуратором `/DumpIB` под замком источника, который
//! берётся только на время снимка и в метку источника ничего не пишет
//! (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`). Новую базу из образа создаёт у файловой
//! цели `ibcmd infobase restore --create-database`, в кластере — Конфигуратор: `CREATEINFOBASE`,
//! затем `/RestoreIB`. Сеансы источника раннер не завершает: неудавшийся снимок называет, как
//! освободить источник (`INV.CLI.A-FAILED-SNAPSHOT-NAMES-THE-RECIPE`). Память новой базы —
//! только признак копии с её поколением (`INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::model::{is_infobase_name, AppConfig};
use crate::domain::capability::{Provider, TargetKind};
use crate::domain::init::InitSource;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::secrets::mask_text;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, CapabilityReason};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::exchange_guard::{
    remember_copied_base, remember_created_base, CopiedFrom, CopiedGeneration,
};
use crate::use_cases::generation_reader::{read_generation, GenerationProcess};
use crate::use_cases::ibcmd_diagnostics::format_failure_evidence;
use crate::use_cases::infobase_lock::{acquire_infobase_lock, BaseAccess};
use crate::use_cases::interruption::collecting_deferrals;
use crate::use_cases::progress::log_live_stage;

use super::{
    cluster_create_failure, cluster_creation, ensure_created, existing_file_infobase,
    infobase_marker_path, interruption_step_outcome, prepare_infobase_parent,
    standalone_refusal, StepOutcome, INFOBASE_CREATE,
};

/// Каталог снимков источников под `workPath`.
const SNAPSHOTS_DIR: &str = "copies";

/// Шаг создания базы копией `from` и источник для ответа: база и путь снимка. Источника нет,
/// пока база-источник не найдена.
pub(super) fn ensure_copy(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    from: &str,
    dry_run: bool,
) -> (StepOutcome, Option<InitSource>) {
    let started = Instant::now();
    let failed = |error: AppError| StepOutcome::failed("infobase", "create", started, error);
    let source = match source_config(config, from) {
        Ok(source) => source,
        Err(error) => return (failed(error), None),
    };
    let snapshot = snapshot_path(config, from);
    let answer = Some(InitSource {
        infobase: from.to_owned(),
        snapshot: snapshot.clone(),
    });
    let copy = Copy {
        context,
        config,
        source: &source,
        from,
        snapshot: &snapshot,
        started,
    };
    (copy.run(utilities, dry_run), answer)
}

/// Конфигурация источника: та же, что у команды, с секцией базы `from` из местного слоя.
/// Копировать можно только объявленную базу — у строки соединения нет учётных данных, —
/// и не ту, которую команда создаёт.
fn source_config(config: &AppConfig, from: &str) -> Result<AppConfig, AppError> {
    if !is_infobase_name(from) {
        return Err(AppError::Validation(
            "--from names an infobase declared in v8project.local.yaml by its name, not by a connection string: declare the source under infobases.<name> with its credentials and pass that name".to_owned(),
        ));
    }
    let Some(infobase) = config.infobases.get(from) else {
        let declared = config
            .infobases
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::Validation(format!(
            "--from: the infobase '{from}' is not declared in v8project.local.yaml (declared: {})",
            if declared.is_empty() { "none" } else { &declared }
        )));
    };
    let source = AppConfig {
        infobase: infobase.clone(),
        infobase_name: Some(from.to_owned()),
        ..config.clone()
    };
    let base_path = crate::support::path::absolute_from_current_dir(&config.base_path)
        .unwrap_or_else(|_| config.base_path.clone());
    let same = config.infobase_name.as_deref() == Some(from)
        || source
            .infobase_memory_address(&base_path)
            .is_some_and(|address| config.infobase_memory_address(&base_path) == Some(address));
    if same {
        return Err(AppError::Validation(format!(
            "--from: the infobase '{from}' is the one this command creates; a copy is made into another infobase — point infobases.origin at a new one with `init --infobase <connection string>`"
        )));
    }
    Ok(source)
}

/// Образ источника `workPath/copies/<источник>.dt`.
fn snapshot_path(config: &AppConfig, from: &str) -> PathBuf {
    crate::support::path::absolute_from_current_dir(&config.work_path)
        .unwrap_or_else(|_| config.work_path.clone())
        .join(SNAPSHOTS_DIR)
        .join(format!("{from}.dt"))
}

/// Отказ источнику на автономном сервере — до снимка и с рецептом снимка на машине сервера
/// (`INV.CLI.A-STANDALONE-SOURCE-IS-REFUSED-WITH-A-RECIPE`).
fn standalone_source_refusal(from: &str) -> AppError {
    AppError::capability_for(
        CapabilityReason::Target,
        format!(
            "infobase create --from does not snapshot the infobase '{from}' of a standalone server: take its DT image on the server machine (`ibcmd infobase dump <file>.dt` against the server's infobase), bring the file here, load it into the infobase of this working copy with `v8-runner infobase restore --input <file>.dt --create`, then load the sources of this working copy over it with `v8-runner push --force`"
        ),
    )
}

/// Как освободить источник, чтобы снимок прошёл. Сеансы раннер сам не завершает, и причину
/// неудачи по прозе платформы не угадывает (`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`): рецепт
/// называется при всякой неудаче снимка.
fn free_the_source(source: &AppConfig, from: &str) -> String {
    match source.target_kind() {
        TargetKind::File => {
            let holders = crate::use_cases::infobase_owner::holders(source)
                .and_then(|holders| holders.owners)
                .map(|owners| {
                    owners
                        .iter()
                        .map(|owner| format!("'{}'", owner.project.display()))
                        .collect::<Vec<_>>()
                })
                .filter(|projects| !projects.is_empty())
                .map(|projects| format!(" ({})", projects.join(", ")))
                .unwrap_or_default();
            format!(
                "the snapshot needs the infobase '{from}' free: close the Designer and the clients of the working copy that holds it{holders} and run the command again; the runner ends no sessions itself"
            )
        }
        TargetKind::Cluster | TargetKind::Standalone => format!(
            "the snapshot needs the infobase '{from}' free: open a maintenance window in its cluster — deny new sessions and terminate the running ones (`sessions deny`, `sessions terminate`) — and run the command again; after the snapshot, whether it succeeded or not, allow sessions again (`sessions allow`); the runner ends no sessions itself"
        ),
    }
}

/// Одна копия: источник, его образ и цель команды.
struct Copy<'a> {
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    source: &'a AppConfig,
    from: &'a str,
    snapshot: &'a Path,
    started: Instant,
}

/// Чем создаётся база по виду цели.
enum Creator {
    /// Файловая база: `ibcmd infobase restore --create-database` в этот каталог.
    File { dir: PathBuf, ibcmd: PathBuf },
    /// База в кластере: Конфигуратор `CREATEINFOBASE`, затем `/RestoreIB`.
    Cluster,
}

impl Copy<'_> {
    fn failed(&self, error: AppError) -> StepOutcome {
        StepOutcome::failed("infobase", "create", self.started, error)
    }

    fn run(&self, utilities: &mut PlatformUtilities, dry_run: bool) -> StepOutcome {
        if self.source.target_kind() == TargetKind::Standalone {
            return self.failed(standalone_source_refusal(self.from));
        }
        let creator = match self.config.target_kind() {
            TargetKind::Standalone => return self.failed(standalone_refusal()),
            TargetKind::Cluster => match cluster_creation(self.config) {
                Ok(_) => Creator::Cluster,
                Err(error) => return self.failed(error),
            },
            TargetKind::File => {
                let Some(dir) = self.config.v8_connection().file_path().map(PathBuf::from) else {
                    return self.failed(AppError::Runtime(
                        "a file target names no infobase path".to_owned(),
                    ));
                };
                if infobase_marker_path(&dir).exists() {
                    return self.failed(existing_file_infobase(&dir));
                }
                match utilities.locate(UtilityType::Ibcmd) {
                    Ok(location) => Creator::File {
                        dir,
                        ibcmd: location.path,
                    },
                    Err(error) => return self.failed(AppError::from(error)),
                }
            }
        };
        let designer = match utilities.locate(UtilityType::V8) {
            Ok(location) => location.path,
            Err(error) => return self.failed(AppError::from(error)),
        };
        let source_target = self.source.v8_connection().describe_target();
        let target = self.config.v8_connection().describe_target();
        if dry_run {
            let creation = match &creator {
                Creator::File { ibcmd, .. } => format!(
                    "a file infobase {target} from it via {} infobase restore --create-database",
                    ibcmd.display()
                ),
                Creator::Cluster => format!(
                    "{target} in the cluster from it via {} CREATEINFOBASE, then /RestoreIB",
                    designer.display()
                ),
            };
            return StepOutcome::planned(
                "infobase",
                "create",
                self.started,
                format!(
                    "would snapshot the infobase '{}' ({source_target}) to '{}' via {} /DumpIB — the source must be free, the runner ends no sessions — and create {creation}",
                    self.from,
                    self.snapshot.display(),
                    designer.display()
                ),
            );
        }
        if let Some(outcome) = interruption_step_outcome(
            self.context,
            "infobase",
            "create",
            self.started,
            "infobase snapshot",
        ) {
            return outcome;
        }
        if let Creator::File { dir, .. } = &creator {
            if let Err(error) = prepare_infobase_parent(dir) {
                return self.failed(error);
            }
        }
        let lock_warning = match self.take_snapshot(utilities, &designer) {
            Ok(warning) => warning,
            Err(error) => return self.failed(error),
        };
        if let Some(outcome) = interruption_step_outcome(
            self.context,
            "infobase",
            "create",
            self.started,
            INFOBASE_CREATE,
        ) {
            return outcome;
        }
        log_live_stage(
            "init: infobase create",
            "[Platform] creating the infobase from the snapshot",
        );
        let settled = collecting_deferrals(|deferrals| {
            let policy = self
                .context
                .process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
            let generation = match &creator {
                Creator::File { dir, ibcmd } => {
                    let marker = infobase_marker_path(dir);
                    let connection = IbcmdConnection::from_infobase(&self.config.infobase)
                        .map_err(AppError::from)?;
                    let created = IbcmdDsl::new(
                        ibcmd.clone(),
                        connection,
                        utilities.runner_for(UtilityType::Ibcmd),
                        policy,
                    )
                    .infobase_restore_creating(self.snapshot)
                    .map_err(AppError::from)?;
                    deferrals.note_result(INFOBASE_CREATE, &created);
                    ensure_created(&created, &marker)?;
                    self.generation_by_ibcmd(ibcmd, utilities)
                }
                Creator::Cluster => {
                    self.create_in_the_cluster(utilities, &designer, policy, deferrals)?;
                    None
                }
            };
            let copied = CopiedFrom {
                source: self.from.to_owned(),
                snapshot: self.snapshot.to_path_buf(),
                since: chrono::Utc::now().to_rfc3339(),
                generation,
            };
            Ok(StepOutcome::ok(
                "infobase",
                "create",
                self.started,
                format!(
                    "{target} created as a copy of the infobase '{}' from the snapshot '{}'; the first push loads every source-set in full",
                    self.from,
                    self.snapshot.display()
                ),
            )
            .with_warnings(lock_warning.as_slice())
            .with_warnings(remember_copied_base(self.config, &copied).as_slice()))
        });
        match settled {
            Ok((step, warnings)) => step.with_warnings(&warnings),
            Err(error) => self.failed(error),
        }
    }

    /// Снимок источника под его замком. Занятый источник — отказ `InfobaseBusy`; замок, который
    /// не взять по другой причине, — предупреждение: снимок только читает источник. Неудача —
    /// с рецептом, как освободить источник; брошенный образ убирается.
    fn take_snapshot(
        &self,
        utilities: &PlatformUtilities,
        designer: &Path,
    ) -> Result<Option<String>, AppError> {
        let lock = acquire_infobase_lock(self.source, INFOBASE_CREATE, BaseAccess::Reads)?;
        let dir = self.snapshot.parent().unwrap_or(self.snapshot);
        std::fs::create_dir_all(dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the snapshot directory '{}': {error}",
                dir.display()
            ))
        })?;
        remove_snapshot(self.snapshot)?;
        log_live_stage(
            "init: infobase snapshot",
            &format!("[Конфигуратор] taking the snapshot of '{}'", self.from),
        );
        let log = crate::support::temp::platform_logs_dir(&self.config.work_path)
            .map(|dir| dir.join("infobase-copy-snapshot.log"))
            .map_err(|error| {
                AppError::Runtime(format!("failed to create platform logs dir: {error}"))
            })?;
        let taken = DesignerDsl::new(
            designer.to_path_buf(),
            self.source.v8_connection(),
            utilities.runner_for(UtilityType::V8),
            Some(log),
            self.context
                .process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        )
        .dump_infobase(self.snapshot)
        .map_err(AppError::from)?;
        let image = std::fs::metadata(self.snapshot)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0);
        if taken.process.outcome().is_err() || image.is_none() {
            // Брошенный образ — свой артефакт команды: следующая попытка начнёт с чистого места.
            let _ = remove_snapshot(self.snapshot);
            return Err(self.snapshot_failure(&taken, image.is_none()));
        }
        Ok(lock.warning().map(str::to_owned))
    }

    fn snapshot_failure(&self, result: &PlatformCommandResult, no_image: bool) -> AppError {
        let secrets: Vec<&str> = [
            self.source.infobase.password.as_deref(),
            self.source
                .infobase
                .dbms
                .as_ref()
                .and_then(|dbms| dbms.password.as_deref()),
        ]
        .into_iter()
        .flatten()
        .filter(|secret| !secret.is_empty())
        .collect();
        let headline = if result.process.outcome().is_err() {
            format!(
                "the snapshot of the infobase '{}' failed with exit code {}",
                self.from, result.process.exit_code
            )
        } else if no_image {
            format!(
                "the snapshot of the infobase '{}' left no image at '{}'",
                self.from,
                self.snapshot.display()
            )
        } else {
            format!("the snapshot of the infobase '{}' failed", self.from)
        };
        let mut message = format_failure_evidence(
            headline,
            &mask_text(&result.process.stdout, &secrets),
            &mask_text(&result.process.stderr, &secrets),
            result
                .platform_log
                .as_deref()
                .map(|log| mask_text(log, &secrets))
                .as_deref(),
            result.platform_log_path.as_deref(),
        );
        message.push_str("; ");
        message.push_str(&free_the_source(self.source, self.from));
        AppError::Platform(message)
    }

    /// Поколение основной конфигурации новой файловой базы — тем же `ibcmd`. Без ответа память
    /// знает только, что база — копия.
    fn generation_by_ibcmd(
        &self,
        ibcmd: &Path,
        utilities: &PlatformUtilities,
    ) -> Option<CopiedGeneration> {
        let read = read_generation(
            self.context,
            self.config,
            || {
                Ok(GenerationProcess::Ibcmd {
                    binary: ibcmd,
                    runner: utilities.runner_for(UtilityType::Ibcmd),
                    data_path: None,
                })
            },
            None,
        );
        match read {
            Ok(Some(token)) => Some(CopiedGeneration {
                tool: Provider::Ibcmd,
                token,
            }),
            Ok(None) => None,
            Err(error) => {
                tracing::debug!(%error, "the generation of the copied infobase was not read");
                None
            }
        }
    }

    /// База в кластере: `CREATEINFOBASE` создаёт её пустой, `/RestoreIB` загружает образ.
    /// Неудачная загрузка оставляет базу созданной пустой: память говорит это, и отказ
    /// называет, как загрузить образ снова.
    fn create_in_the_cluster(
        &self,
        utilities: &PlatformUtilities,
        designer: &Path,
        policy: crate::platform::process::ProcessExecutionPolicy,
        deferrals: &mut crate::use_cases::interruption::Deferrals,
    ) -> Result<(), AppError> {
        let creation = cluster_creation(self.config)?;
        let database = format!(
            "the database '{}' on '{}'",
            creation.database_name, creation.database_server
        );
        let created = DesignerDsl::new(
            designer.to_path_buf(),
            self.config.v8_connection(),
            utilities.runner_for(UtilityType::V8),
            None,
            policy.clone(),
        )
        .create_cluster_infobase(&creation)
        .map_err(AppError::from)?;
        deferrals.note_result(INFOBASE_CREATE, &created);
        if created.process.outcome().is_err() {
            return Err(cluster_create_failure(&creation, &created, &database));
        }
        let log = crate::support::temp::platform_logs_dir(&self.config.work_path)
            .map(|dir| dir.join("infobase-copy-restore.log"))
            .map_err(|error| {
                AppError::Runtime(format!("failed to create platform logs dir: {error}"))
            })?;
        let restored = DesignerDsl::new(
            designer.to_path_buf(),
            self.config.v8_connection(),
            utilities.runner_for(UtilityType::V8),
            Some(log),
            policy,
        )
        .restore_infobase(self.snapshot);
        let failure = match restored {
            Ok(result) => {
                deferrals.note_result(INFOBASE_CREATE, &result);
                match result.process.outcome() {
                    Ok(()) => return Ok(()),
                    Err(_) => AppError::Platform(super::failure_details(
                        "load the snapshot",
                        "infobase",
                        &result,
                    )),
                }
            }
            Err(error) => AppError::from(error),
        };
        let memory = remember_created_base(self.config, None)
            .map(|failure| format!(" ({failure})"))
            .unwrap_or_default();
        Err(failure.with_context(format!(
            "the infobase was created empty in the cluster and the snapshot was not loaded into it{memory}: load it with `v8-runner infobase restore --input {} --replace`, or load the sources with the first push, which goes in full",
            self.snapshot.display()
        )))
    }
}

/// Убирает образ по пути снимка: прежний — до снимка, брошенный — после неудачи.
fn remove_snapshot(snapshot: &Path) -> Result<(), AppError> {
    match std::fs::remove_file(snapshot) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Runtime(format!(
            "failed to remove the previous snapshot '{}': {error}",
            snapshot.display()
        ))),
    }
}
