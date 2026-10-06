//! Проверки перед обменом с базой: есть ли о ней память и не ушла ли она вперёд.
//!
//! Чья база, проверяет граница команды (`transport.rs`); здесь — то, что идёт после неё
//! (`INV.USE-CASES.OWNERSHIP-IS-CHECKED-BEFORE-MEMORY-AND-GENERATION`):
//!
//! - `push` без памяти о базе отказывает до загрузки, и для пустой базы тоже: поколение
//!   пустой базы зависит от версии платформы (`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`);
//! - `push` в базу, чьё поколение отличается от записанного тем же инструментом, отказывает
//!   `non_fast_forward` (`INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED`);
//! - после удачной загрузки поколение читается снова и записывается; без ответа запись
//!   набора стирается: прежний токен описывает уже не ту базу;
//! - `pull` записывает поколение, только если оно до и после выгрузки одно и то же.
//!
//! Память о наборе — запись журнала поколений или непустая хеш-память этой пары «база ↔
//! каталог». Признак нового владельца — взял базу без метки или сменил ушедшего — лежит в
//! памяти базы под `workPath` и снимается первой удачной отправкой
//! (`INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::change_detection::analyzer::{self, SnapshotMemory};
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::AppConfig;
use crate::domain::capability::{Provider, TargetKind};
use crate::domain::next_step::NextStep;
use crate::domain::source_set::SourceSetContext;
use crate::support::error::{AppError, NonFastForward};
use crate::use_cases::agent_session::{
    GenerationAfter, GenerationComparison, GenerationLedger, Recorded,
};
use crate::use_cases::context::{shell_word, ExecutionContext};
use crate::use_cases::ignored_files::VERSION_FILE_NAME;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Файл признака нового владельца под памятью базы.
const NEW_OWNER_FILE_NAME: &str = "new-owner.json";

/// Признак нового владельца: когда копия взяла базу.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NewOwnerMark {
    since: String,
}

fn new_owner_file(config: &AppConfig) -> Option<PathBuf> {
    SourceSetsService::new(config)
        .base_memory_dir()
        .map(|dir| dir.join(NEW_OWNER_FILE_NAME))
}

/// Записывает признак нового владельца: копия взяла базу без метки или сменила ушедшего
/// владельца. Зовёт его проверка владельца под замками базы и `workPath`.
pub(crate) fn remember_new_owner(config: &AppConfig) -> Result<(), String> {
    let Some(file) = new_owner_file(config) else {
        return Ok(());
    };
    let text = serde_json::to_vec(&NewOwnerMark {
        since: chrono::Utc::now().to_rfc3339(),
    })
    .map_err(|error| error.to_string())?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    crate::support::fs::write_file_atomically(&file, |out| std::io::Write::write_all(out, &text))
        .map_err(|error| format!("'{}': {error}", file.display()))
}

/// Снимает признак нового владельца: первая удачная отправка или созданная раннером база.
/// Несостоявшееся снятие — предупреждение: признак лишь сужает выходы будущих отказов.
pub(crate) fn forget_new_owner(config: &AppConfig) -> Option<String> {
    let file = new_owner_file(config)?;
    match std::fs::remove_file(&file) {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(format!(
            "the mark that this working copy took the infobase over was not removed from '{}': {error}; refusals keep offering no pull",
            file.display()
        )),
    }
}

/// Когда копия взяла базу, если она — новый владелец до первой удачной отправки.
fn new_owner_since(config: &AppConfig) -> Option<String> {
    let file = new_owner_file(config)?;
    let text = std::fs::read(&file).ok()?;
    // Признак, который не разобрать, всё равно признак: выгрузку не предлагаем.
    Some(
        serde_json::from_slice::<NewOwnerMark>(&text)
            .map(|record| record.since)
            .unwrap_or_else(|_| "at an unknown time".to_owned()),
    )
}

/// Положение базы, от которого зависят выходы отказа.
struct Standing {
    /// Копия взяла базу без метки или сменила ушедшего владельца и ещё не отправляла.
    new_owner: Option<String>,
    /// База в кластере или на автономном сервере: метки у неё нет.
    server: bool,
    /// Общая база (`shared: true`): остальные её владельцы из метки.
    shared_with: Option<Vec<String>>,
}

impl Standing {
    fn of(config: &AppConfig) -> Self {
        Self {
            new_owner: new_owner_since(config),
            server: config.target_kind() != TargetKind::File,
            shared_with: crate::use_cases::infobase_owner::shared_base_owners(config),
        }
    }

    /// Выгрузку предлагают всем, кроме нового владельца до первой отправки. Общей базе
    /// (`shared: true`) — всегда: решение владельца продукта от 06.10.2026 даёт ей оба выхода,
    /// и её меняют другие копии по согласию, а не захват.
    fn offers_pull(&self) -> bool {
        self.shared_with.is_some() || self.new_owner.is_none()
    }

    /// Почему выгрузка не предложена и кто ещё мог менять базу.
    fn caveats(&self) -> String {
        let mut text = String::new();
        if let Some(others) = &self.shared_with {
            let others = if others.is_empty() {
                "none recorded in the owner marker".to_owned()
            } else {
                others.join(", ")
            };
            text.push_str(&format!(
                " The infobase is shared (`shared: true`): other working copies change it too; its other owners: {others}."
            ));
        } else if let Some(since) = &self.new_owner {
            text.push_str(&format!(
                " This working copy took the infobase over ({since}) and has not pushed into it since: other working copies may have changed it, and taking its state into this directory would bring their work here, so only the overwrite is offered."
            ));
        }
        if self.server {
            text.push_str(
                " A server infobase has no owner marker: another working copy may have changed it, and only the generation check protects it.",
            );
        }
        text
    }
}

/// Отказ `push` без памяти о базе. Наборы `contexts` — те, что пойдут в базу; каждому
/// нужна своя память: запись журнала поколений или непустая хеш-память этой пары.
/// Чужую хеш-память называет свой отказ анализа изменений
/// (`INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED`), поэтому здесь отсутствием памяти она не считается.
pub(crate) fn require_memory(
    context: &ExecutionContext,
    config: &AppConfig,
    contexts: &[SourceSetContext],
    selected_set: Option<&str>,
) -> Result<(), UseCaseError> {
    let forgotten: Vec<&SourceSetContext> = contexts
        .iter()
        .filter(|set| !remembers(set, &config.work_path))
        .collect();
    let Some(first) = forgotten.first() else {
        return Ok(());
    };
    let standing = Standing::of(config);
    let names = forgotten
        .iter()
        .map(|set| format!("'{}'", set.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let target = config.v8_connection().describe_target();
    let push_force = context.advised_command(&match selected_set {
        Some(set) => format!("push {} --force", shell_word(set)),
        None => "push --force".to_owned(),
    });
    let overwrite = format!(
        "{push_force}, which loads the whole directory and replaces the configuration in the infobase, losing what was changed there"
    );
    // Базу с нераспознанным адресом раннер не помнит никогда: выход у неё один.
    let unrememberable = first.storage_identity().is_none();
    let mut message = if unrememberable {
        format!(
            "cannot push: the runner keeps no memory of {target}, because it does not recognize its address, so nothing proves that the sources derive from its state; to load them anyway run {overwrite}"
        )
    } else {
        format!(
            "cannot push: this working copy has no memory of {target} for source-set {names}, so nothing proves that the sources derive from its state; an empty infobase is not told apart, because its generation depends on the platform version"
        )
    };
    let offers_pull = standing.offers_pull() && !unrememberable;
    if !unrememberable {
        if offers_pull {
            let pulls = forgotten
                .iter()
                .map(|set| context.advised_command(&format!("pull {}", shell_word(set.name()))))
                .collect::<Vec<_>>()
                .join(", ");
            message.push_str(&format!(
                ". If the infobase holds the right state, see what is there with {pulls}; if the source directory does, run {overwrite}"
            ));
        } else {
            message.push_str(&format!(". To load the sources run {overwrite}"));
        }
        message.push('.');
        message.push_str(&standing.caveats());
    }
    let next = if offers_pull {
        NextStep::command("pull").for_source_set(first.name())
    } else {
        let next = NextStep::command("push").with_key("--force", "");
        match selected_set {
            Some(set) => next.for_source_set(set),
            None => next,
        }
    };
    Err(UseCaseError::new(UseCaseErrorKind::NoMemory, message).with_next(next))
}

/// Помнит ли рабочая копия базу для набора: своя запись поколения или своя непустая
/// хеш-память. Чужая и нечитаемая хеш-память отвечают своими отказами дальше.
fn remembers(set: &SourceSetContext, work_path: &Path) -> bool {
    let recorded = GenerationLedger::of(set, work_path)
        .is_some_and(|ledger| matches!(ledger.read(), Recorded::Ours(_)));
    recorded
        || matches!(
            analyzer::snapshot_memory(set, work_path),
            SnapshotMemory::Own | SnapshotMemory::Foreign | SnapshotMemory::Unreadable
        )
}

/// Что показало поколение перед загрузкой.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BeforeLoad {
    /// Сверки не было: `--force`, записи того же инструмента нет или нет ответа.
    Unchecked,
    /// Поколение совпало с записанным тем же инструментом.
    Matched,
}

/// Сверка поколения у отправки: перед загрузкой набора и после неё.
pub(crate) struct GenerationGate<'a> {
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    force: bool,
}

impl<'a> GenerationGate<'a> {
    pub(crate) fn new(context: &'a ExecutionContext, config: &'a AppConfig, force: bool) -> Self {
        Self {
            context,
            config,
            force,
        }
    }

    /// Перед загрузкой набора: поколение базы, прочитанное тем же инструментом, что записал
    /// прошлое, отличается от записанного — отказ `non_fast_forward` до загрузки. Без записи
    /// того же инструмента поколение не читается: сравнивать не с чем.
    pub(crate) fn before_load(
        &self,
        set: &SourceSetContext,
        tool: Provider,
        read: impl FnOnce() -> Result<Option<String>, AppError>,
    ) -> Result<BeforeLoad, AppError> {
        // После отмены поколение не спрашивают: загрузку остановит её безопасная точка.
        if self.force
            || crate::use_cases::interruption::pending_interruption_error(
                self.context,
                "the configuration generation",
            )
            .is_some()
        {
            return Ok(BeforeLoad::Unchecked);
        }
        let Some(record) = GenerationLedger::of(set, &self.config.work_path).and_then(|ledger| {
            match ledger.read() {
                Recorded::Ours(record) if record.tool == tool => Some(record),
                Recorded::Ours(_) | Recorded::Nothing | Recorded::Foreign { .. } => None,
            }
        }) else {
            return Ok(BeforeLoad::Unchecked);
        };
        let Some(token) = read()? else {
            return Ok(BeforeLoad::Unchecked);
        };
        match record.compare(tool, &token) {
            GenerationComparison::Unchanged => Ok(BeforeLoad::Matched),
            GenerationComparison::NoAnswer => Ok(BeforeLoad::Unchecked),
            GenerationComparison::Changed => Err(self.moved_ahead(
                set.name(),
                &token,
                &record.token,
                &record.after.to_string(),
                &record.recorded_at,
            )),
        }
    }

    fn moved_ahead(
        &self,
        set: &str,
        base: &str,
        local: &str,
        after: &str,
        recorded_at: &str,
    ) -> AppError {
        let standing = Standing::of(self.config);
        let target = self.config.v8_connection().describe_target();
        let push_force = self
            .context
            .advised_command(&format!("push {} --force", shell_word(set)));
        let mut message = format!(
            "cannot push source-set '{set}': {target} moved ahead since the last exchange of this working copy — its configuration generation is {base}, the one recorded after the last {after} ({recorded_at}) is {local}"
        );
        if standing.offers_pull() {
            message.push_str(&format!(
                "; take its changes first with {}, or overwrite them with {push_force}",
                self.context
                    .advised_command(&format!("pull {}", shell_word(set)))
            ));
        } else {
            message.push_str(&format!("; to overwrite them run {push_force}"));
        }
        message.push('.');
        message.push_str(&standing.caveats());
        AppError::NonFastForward(Box::new(NonFastForward {
            message,
            source_set: set.to_owned(),
            base_generation: base.to_owned(),
            local_generation: local.to_owned(),
            offers_pull: standing.offers_pull(),
        }))
    }

    /// После удачной загрузки набора: поколение записывается с инструментом. Без ответа
    /// запись набора стирается — прежний токен описывает уже не ту базу, и следующая
    /// отправка не должна принять свою же загрузку за чужую правку. Загрузка уже прошла,
    /// поэтому сбой записи — предупреждение, а не отказ.
    #[must_use]
    pub(crate) fn after_load(
        &self,
        set: &SourceSetContext,
        tool: Provider,
        token: Option<&str>,
    ) -> Option<String> {
        let ledger = GenerationLedger::of(set, &self.config.work_path)?;
        let written = match token {
            Some(token) => ledger.record(tool, token, GenerationAfter::Build),
            None => ledger.forget(),
        };
        written.err().map(|error| {
            let _ = ledger.forget();
            format!(
                "the configuration generation of source-set '{}' was not recorded: {error}; the next push does not check whether the infobase moved ahead",
                set.name()
            )
        })
    }

    /// Восстановление потерянного файла версий без полной выгрузки
    /// (`INV.USE-CASES.A-VERSION-FILE-ALONE-IS-DUMPED-ONLY-WHEN-THE-DIRECTORY-MATCHES-THE-BASE`).
    ///
    /// Сразу после удачной загрузки, если файла версий в каталоге набора нет, а совпадение
    /// каталога и базы доказано — загрузка была полной или поколение перед частичной совпало
    /// с записанным, — выгружается один файл версий: `dump_and_reread` выгружает его и
    /// читает поколение снова, а `None` значит, что инструмент этого не умеет. Поколение после
    /// загрузки и после выгрузки сравнивается тем же инструментом: изменилось или ответа нет
    /// — файл убирается. Возвращает строку для ответа.
    pub(crate) fn restore_version_file(
        &self,
        set: &SourceSetContext,
        full: bool,
        before: &BeforeLoad,
        token: Option<&str>,
        dump_and_reread: impl FnOnce() -> Option<Result<Option<String>, AppError>>,
    ) -> Option<String> {
        let file = set.path().join(VERSION_FILE_NAME);
        let proven = full || *before == BeforeLoad::Matched;
        let token = token?;
        if file.exists()
            || !proven
            || crate::use_cases::interruption::pending_interruption_error(
                self.context,
                "the configuration generation",
            )
            .is_some()
        {
            return None;
        }
        match dump_and_reread()? {
            Ok(Some(after)) if after == token && file.is_file() => Some(format!(
                "{VERSION_FILE_NAME} was missing from source-set '{}' and was dumped alone, without a full dump: the directory matches the infobase",
                set.name()
            )),
            Ok(_) | Err(_) => {
                let _ = std::fs::remove_file(&file);
                Some(format!(
                    "{VERSION_FILE_NAME} is missing from source-set '{}' and was not restored: the configuration generation did not confirm that the infobase stayed unchanged while it was dumped; the next pull runs full",
                    set.name()
                ))
            }
        }
    }
}

/// Поколение у выгрузки: до и после. Совпало — запись после выгрузки; изменилось —
/// ответ называет это, а память не обновляется, и следующая отправка снова увидит
/// расхождение. Без ответа до или после памяти нечего записать.
pub(crate) fn record_after_dump(
    set: &SourceSetContext,
    work_path: &Path,
    tool: Provider,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<Option<String>, AppError> {
    let (Some(before), Some(after)) = (before, after) else {
        return Ok(None);
    };
    let Some(ledger) = GenerationLedger::of(set, work_path) else {
        return Ok(None);
    };
    if before == after {
        ledger.record(tool, after, GenerationAfter::Dump)?;
        return Ok(None);
    }
    Ok(Some(format!(
        "the infobase was changed while source-set '{}' was being dumped: its configuration generation was {before} before the dump and {after} after it; the memory of the infobase is not updated, so the next push sees the difference — pull again",
        set.name()
    )))
}

/// Память о базе, которую раннер только что создал пустой: у каждого набора, который в неё
/// пойдёт, — пустая хеш-память этой пары, и первая отправка грузит набор целиком без отказа
/// первого знакомства. Признак нового владельца снимается: база своя с рождения. Сбой —
/// строка для ответа: база создана, а первая отправка без памяти откажет и назовёт выходы.
pub(crate) fn remember_created_base(config: &AppConfig) -> Option<String> {
    let failures: Vec<String> = SourceSetsService::new(config)
        .designer_contexts()
        .iter()
        .filter(|set| set.storage_identity().is_some())
        .filter_map(|set| {
            analyzer::commit_empty_snapshot(set, &config.work_path)
                .err()
                .map(|error| format!("source-set '{}': {error}", set.name()))
        })
        .chain(forget_new_owner(config))
        .collect();
    (!failures.is_empty()).then(|| {
        format!(
            "the memory of the created infobase was not written ({}); the first push names the ways out",
            failures.join("; ")
        )
    })
}

/// Для тестов сценариев: память о базе у каждого набора, о котором её ещё нет, — запись
/// журнала поколений, которую поддельный исполнитель не подтвердит и не опровергнет. Хеш-память
/// теста она не трогает.
#[cfg(test)]
pub(crate) fn remember_unknown_sets(config: &AppConfig) {
    for set in SourceSetsService::new(config).designer_contexts() {
        if let Some(ledger) = GenerationLedger::of(&set, &config.work_path) {
            if !remembers(&set, &config.work_path) {
                ledger
                    .record(Provider::Designer, &"0".repeat(40), GenerationAfter::Build)
                    .expect("generation record");
            }
        }
    }
}

/// Каталог памяти о базах `workPath/infobases`, в котором нет ничего, кроме признаков нового
/// владельца. Так его оставляет граница `clone` до записи проекта: проверка пустоты каталога
/// клона его не считает.
pub(crate) fn holds_only_new_owner_marks(infobases: &Path) -> bool {
    let Ok(bases) = std::fs::read_dir(infobases) else {
        return false;
    };
    bases.flatten().all(|base| {
        std::fs::read_dir(base.path()).is_ok_and(|entries| {
            entries
                .flatten()
                .all(|entry| entry.file_name() == NEW_OWNER_FILE_NAME)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{
        InfobaseConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::use_cases::context::CommandName;

    fn project(root: &Path) -> AppConfig {
        std::fs::create_dir_all(root.join("main")).expect("sources");
        AppConfig {
            base_path: root.to_path_buf(),
            work_path: root.join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file(format!("File={}", root.join("ib").display())),
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("main"),
            }],
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn require(config: &AppConfig) -> Result<(), UseCaseError> {
        require_memory(
            &ExecutionContext::cli(CommandName::Build),
            config,
            &SourceSetsService::new(config).designer_contexts(),
            None,
        )
    }

    /// Базу, созданную раннером, он помнит: первая отправка в неё без отказа первого
    /// знакомства, а признак нового владельца снят.
    #[test]
    fn a_base_created_by_the_runner_is_remembered() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = project(root.path());
        remember_new_owner(&config).expect("new owner mark");
        assert_eq!(
            require(&config).expect_err("no memory yet").kind(),
            UseCaseErrorKind::NoMemory
        );

        assert_eq!(remember_created_base(&config), None);

        require(&config).expect("the created base is remembered");
        assert_eq!(new_owner_since(&config), None);
    }

    /// Признак нового владельца не считается содержимым каталога клона.
    #[test]
    fn a_new_owner_mark_alone_leaves_the_memory_empty_for_a_clone() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = project(root.path());
        remember_new_owner(&config).expect("new owner mark");

        assert!(holds_only_new_owner_marks(
            &root.path().join("work/infobases")
        ));
        remember_created_base(&config);
        assert!(!holds_only_new_owner_marks(
            &root.path().join("work/infobases")
        ));
    }
}
