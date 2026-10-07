//! `infobase create --from <база>`: база этой рабочей копии — копия другой объявленной базы с
//! её данными и конфигурацией (`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).
//!
//! С источника снимается образ DT Конфигуратором `/DumpIB` — исполнителем `infobase dump`
//! ([`run_snapshot_provider`]). Замок источника берёт граница команды
//! ([`crate::use_cases::transport::hold_source_base`]) и держит до её конца; в метку источника
//! ничего не пишется (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`). Новую базу из образа
//! поднимает Конфигуратор `/RestoreIB` — исполнитель `infobase restore`
//! ([`run_restore_provider`]): файловую, которой нет, он создаёт сам (замер «Загрузка
//! информационной базы из DT»), в кластере её прежде создаёт `CREATEINFOBASE` —
//! [`ClusterCreation`], общий с `infobase create`. Сеансы источника раннер не завершает:
//! неудавшийся снимок называет, как освободить источник
//! (`INV.CLI.A-FAILED-SNAPSHOT-NAMES-THE-RECIPE`). Память новой базы — только признак копии
//! (`INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::model::{is_infobase_name, AppConfig};
use crate::domain::capability::{Provider, TargetKind};
use crate::domain::init::InitSource;
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::secrets::mask_text;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, CapabilityReason};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::exchange_guard::{remember_copied_base, remember_created_base, CopiedFrom};
use crate::use_cases::generation_reader::{designer_log_file, read_generation, GenerationProcess};
use crate::use_cases::infobase_export::{
    run_restore_provider, run_snapshot_provider, validate_platform_artifact,
};
use crate::use_cases::interruption::collecting_deferrals;
use crate::use_cases::progress::log_live_stage;

use super::{
    ensure_created, existing_file_infobase, failure_details, infobase_marker_path,
    infobase_secrets, interruption_step_outcome, locate_infobase_creator, masked_evidence,
    prepare_infobase_parent, standalone_refusal, ClusterCreation, StepOutcome, INFOBASE_CREATE,
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
    let source = match source_config(config, from) {
        Ok(source) => source,
        Err(error) => {
            return (
                StepOutcome::failed("infobase", "create", started, error),
                None,
            )
        }
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
/// и не ту, которую команда создаёт. По ней же граница команды берёт замок источника.
pub(crate) fn source_config(config: &AppConfig, from: &str) -> Result<AppConfig, AppError> {
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
            if declared.is_empty() {
                "none"
            } else {
                &declared
            }
        )));
    };
    let source = AppConfig {
        infobase: infobase.clone(),
        infobase_name: Some(from.to_owned()),
        ..config.clone()
    };
    let base_path = absolute(&config.base_path);
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

fn absolute(path: &Path) -> PathBuf {
    crate::support::path::absolute_from_current_dir(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Образ источника `workPath/copies/<источник>.dt`.
fn snapshot_path(config: &AppConfig, from: &str) -> PathBuf {
    absolute(&config.work_path)
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

/// Файлового источника нет на месте: снимать нечего, и рецепт про сеансы тут не поможет.
fn missing_file_source(from: &str, dir: &Path) -> AppError {
    AppError::Validation(format!(
        "--from: the file infobase '{from}' is not found at '{}' (no 1Cv8.1CD there): check infobases.{from}.connection in v8project.local.yaml",
        dir.display()
    ))
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

/// Убирает брошенный образ — свой артефакт команды: следующая попытка начнёт с чистого
/// места. Неудача уборки ответ не меняет и уходит в журнал.
fn discard_snapshot(snapshot: &Path) {
    match std::fs::remove_file(snapshot) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            snapshot = %snapshot.display(),
            %error,
            "the abandoned snapshot was not removed"
        ),
    }
}

/// Почему снимок не удался.
enum SnapshotFailure {
    /// Конфигуратор ответил ненулевым кодом.
    Exited,
    /// Код нулевой, а образа нет или он пуст.
    LeftNoImage(AppError),
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
enum Creator<'a> {
    /// Файловая база: `/RestoreIB` создаёт её в этом каталоге.
    File { dir: PathBuf },
    /// База в кластере: `CREATEINFOBASE`, затем `/RestoreIB`.
    Cluster(ClusterCreation<'a>),
}

impl Copy<'_> {
    fn failed(&self, error: AppError) -> StepOutcome {
        StepOutcome::failed("infobase", "create", self.started, error)
    }

    fn run(&self, utilities: &mut PlatformUtilities, dry_run: bool) -> StepOutcome {
        match self.source.target_kind() {
            TargetKind::Standalone => return self.failed(standalone_source_refusal(self.from)),
            TargetKind::File => {
                let dir = self
                    .source
                    .v8_connection()
                    .file_infobase_dir(&absolute(&self.source.base_path));
                if let Some(dir) = dir.filter(|dir| !infobase_marker_path(dir).exists()) {
                    return self.failed(missing_file_source(self.from, &dir));
                }
            }
            TargetKind::Cluster => {}
        }
        let creator = match self.config.target_kind() {
            TargetKind::Standalone => return self.failed(standalone_refusal()),
            TargetKind::Cluster => match ClusterCreation::of(self.config) {
                Ok(cluster) => Creator::Cluster(cluster),
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
                Creator::File { dir }
            }
        };
        // Конфигуратор ищется так же, как у `infobase create`: превью отказывает на той же
        // отсутствующей платформе, что и прогон.
        let designer = match locate_infobase_creator(Provider::Designer, utilities) {
            Ok(binary) => binary,
            Err(error) => return self.failed(error),
        };
        if dry_run {
            return StepOutcome::planned(
                "infobase",
                "create",
                self.started,
                self.plan(&creator, &designer),
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
        if let Creator::File { dir } = &creator {
            if let Err(error) = prepare_infobase_parent(dir) {
                return self.failed(error);
            }
        }
        if let Err(error) = self.take_snapshot(&designer) {
            return self.failed(error);
        }
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
            let mut warnings = Vec::new();
            let target = self.config.v8_connection().describe_target();
            match &creator {
                Creator::File { dir } => {
                    let created = self.restore(&designer)?;
                    deferrals.note_result(INFOBASE_CREATE, &created);
                    let mut secrets = infobase_secrets(self.config);
                    secrets.extend(infobase_secrets(self.source));
                    ensure_created(&created, &infobase_marker_path(dir), &secrets)?;
                    warnings.extend(self.probe_access(utilities, &designer));
                }
                Creator::Cluster(cluster) => {
                    cluster.create(self.context, self.config, utilities, deferrals)?;
                    let restored = self.restore(&designer);
                    if let Ok(result) = &restored {
                        deferrals.note_result(INFOBASE_CREATE, result);
                    }
                    self.loaded_into_the_cluster(cluster, restored)?;
                    warnings.push(format!(
                        "{}; if that database existed, its data is now replaced by the image of '{}'",
                        cluster.risk(),
                        self.from
                    ));
                }
            }
            let copied = CopiedFrom {
                source: self.from.to_owned(),
                snapshot: self.snapshot.to_path_buf(),
                since: chrono::Utc::now(),
            };
            warnings.extend(remember_copied_base(self.config, &copied));
            Ok(StepOutcome::ok(
                "infobase",
                "create",
                self.started,
                format!(
                    "{target} created as a copy of the infobase '{}' from the snapshot '{}'; its infobase users are those of the source; the first push loads every source-set in full",
                    self.from,
                    self.snapshot.display()
                ),
            )
            .with_warnings(&warnings))
        });
        match settled {
            Ok((step, warnings)) => step.with_warnings(&warnings),
            Err(error) => self.failed(error),
        }
    }

    /// План превью: снимок и создание; у кластера — с предупреждением, что существующая база
    /// данных с тем же именем будет перезаписана образом.
    fn plan(&self, creator: &Creator<'_>, designer: &Path) -> String {
        let creation = match creator {
            Creator::File { .. } => format!(
                "{} from it via {} /RestoreIB",
                self.config.v8_connection().describe_target(),
                designer.display()
            ),
            Creator::Cluster(cluster) => format!(
                "{}; then /RestoreIB loads the image into it, and the data of an existing database of that name is overwritten by the image of '{}'",
                cluster.plan(designer),
                self.from
            ),
        };
        format!(
            "would snapshot the infobase '{}' ({}) to '{}' via {} /DumpIB — the source must be free, the runner ends no sessions — and create {creation}",
            self.from,
            self.source.v8_connection().describe_target(),
            self.snapshot.display(),
            designer.display()
        )
    }

    /// Снимок источника исполнителем `infobase dump`. Неудача — с рецептом, как освободить
    /// источник; брошенный образ убирается.
    fn take_snapshot(&self, designer: &Path) -> Result<(), AppError> {
        let dir = self.snapshot.parent().unwrap_or(self.snapshot);
        std::fs::create_dir_all(dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the snapshot directory '{}': {error}",
                dir.display()
            ))
        })?;
        match std::fs::remove_file(self.snapshot) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Runtime(format!(
                    "failed to remove the previous snapshot '{}': {error}",
                    self.snapshot.display()
                )))
            }
        }
        log_live_stage(
            "init: infobase snapshot",
            &format!("[Конфигуратор] taking the snapshot of '{}'", self.from),
        );
        let taken = run_snapshot_provider(
            self.context,
            self.source,
            Provider::Designer,
            Some(designer),
            self.snapshot,
        )
        .inspect_err(|_| discard_snapshot(self.snapshot))?;
        let failure = if taken.process.outcome().is_err() {
            Some(SnapshotFailure::Exited)
        } else {
            validate_platform_artifact(self.snapshot)
                .err()
                .map(SnapshotFailure::LeftNoImage)
        };
        match failure {
            None => Ok(()),
            Some(failure) => {
                discard_snapshot(self.snapshot);
                Err(self.snapshot_failure(&taken, failure))
            }
        }
    }

    fn snapshot_failure(
        &self,
        result: &PlatformCommandResult,
        failure: SnapshotFailure,
    ) -> AppError {
        let headline = match failure {
            SnapshotFailure::Exited => format!(
                "the snapshot of the infobase '{}' failed with exit code {}",
                self.from, result.process.exit_code
            ),
            SnapshotFailure::LeftNoImage(error) => format!(
                "the snapshot of the infobase '{}' left no image: {}",
                self.from, error
            ),
        };
        let mut message = masked_evidence(headline, result, &infobase_secrets(self.source));
        message.push_str("; ");
        message.push_str(&free_the_source(self.source, self.from));
        AppError::Platform(message)
    }

    /// `/RestoreIB` образа в базу команды — исполнителем `infobase restore`.
    fn restore(&self, designer: &Path) -> Result<PlatformCommandResult, AppError> {
        run_restore_provider(
            self.context,
            self.config,
            Provider::Designer,
            Some(designer),
            self.snapshot,
        )
        .map_err(|failure| failure.into_error(INFOBASE_CREATE))
    }

    /// Открывает ли раннер новую файловую базу: пользователи у неё — пользователи источника, и
    /// без их учётных данных в секции базы команды её не открыть. Вопрос — поколение
    /// конфигурации; ответ не записывается, а его нет — предупреждение.
    fn probe_access(&self, utilities: &PlatformUtilities, designer: &Path) -> Option<String> {
        let read = read_generation(
            self.context,
            self.config,
            || {
                Ok(GenerationProcess::Designer {
                    binary: designer,
                    runner: utilities.runner_for(UtilityType::V8),
                    log_file: designer_log_file(self.config, "infobase-copy-generation")?,
                })
            },
            None,
        );
        let why = match read {
            Ok(Some(_)) => return None,
            Ok(None) => "it gave no configuration generation".to_owned(),
            Err(error) => format!(
                "its configuration generation was not read: {}",
                mask_text(&error.to_string(), &infobase_secrets(self.config))
            ),
        };
        let name = self.config.infobase_name.as_deref().unwrap_or("origin");
        Some(format!(
            "the copied infobase did not answer the runner ({why}); its infobase users are those of '{}': declare the name and password of one of them at infobases.{name} in v8project.local.yaml",
            self.from
        ))
    }

    /// Загрузка образа в созданную базу кластера. Неудача оставляет базу созданной пустой:
    /// память говорит это, и отказ называет, как загрузить образ снова. Пароли базы, её СУБД и
    /// администратора кластера в выводе скрыты.
    fn loaded_into_the_cluster(
        &self,
        cluster: &ClusterCreation<'_>,
        restored: Result<PlatformCommandResult, AppError>,
    ) -> Result<(), AppError> {
        let failure = match restored {
            Ok(result) => match result.process.outcome() {
                Ok(()) => return Ok(()),
                Err(_) => {
                    let mut secrets = infobase_secrets(self.config);
                    secrets.extend(infobase_secrets(self.source));
                    secrets.extend(cluster.secrets());
                    AppError::Platform(failure_details(
                        "load the snapshot",
                        "infobase",
                        &result,
                        &secrets,
                    ))
                }
            },
            Err(error) => error,
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
