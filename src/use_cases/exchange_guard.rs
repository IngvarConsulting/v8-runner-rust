//! Проверки перед обменом с базой: есть ли о ней память и не ушла ли она вперёд.
//!
//! Чья база, проверяет граница команды (`transport.rs`); здесь — то, что идёт после неё
//! (`INV.USE-CASES.OWNERSHIP-IS-CHECKED-BEFORE-MEMORY-AND-GENERATION`):
//!
//! - `push` без памяти о базе отказывает до загрузки, и для пустой базы тоже: поколение
//!   пустой базы зависит от версии платформы (`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`);
//! - `push` в базу, чьё поколение отличается от записанного тем же инструментом, отказывает
//!   `non_fast_forward` (`INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED`);
//!   обходит обе проверки только `push --force` ([`PushMode::Force`]);
//! - после удачной загрузки поколение читается снова и записывается; без ответа запись
//!   набора стирается, и ответ это называет; после неудачной запись помечается как сделанная
//!   перед ней;
//! - `pull` записывает поколение после выгрузки, если оно до и после одно и то же, иначе —
//!   поколение до неё, и следующая отправка видит расхождение.
//!
//! Отказы `no_memory` и `non_fast_forward` и их следующий шаг строит только этот модуль.
//! Память о наборе — своя запись журнала поколений или своя хеш-память этой пары «база ↔
//! каталог», в том числе пустая. Признак нового владельца — взял базу без метки или сменил
//! ушедшего — лежит в памяти базы под `workPath` и снимается первой удачной отправкой
//! (`INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::change_detection::analyzer::{self, SnapshotMemory};
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::AppConfig;
use crate::domain::capability::{Operation, Provider, TargetKind};
use crate::domain::next_step::NextStep;
use crate::domain::source_set::SourceSetContext;
use crate::domain::status::{GenerationAfter, GenerationVerdict, MemoryState};
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    GenerationComparison, GenerationLedger, GenerationRecord, Recorded,
};
use crate::use_cases::context::{shell_word, ExecutionContext};
use crate::use_cases::ignored_files::VERSION_FILE_NAME;
use crate::use_cases::request::PushMode;
use crate::use_cases::result::{Generations, UseCaseError, UseCaseErrorKind};

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
///
/// Признак, который не прочесть или не разобрать, всё равно признак: выгрузку не предлагаем.
/// Так безопаснее — лишний раз не предложенный `pull` стоит меньше, чем предложенная
/// выгрузка чужой работы в этот каталог.
pub(crate) fn new_owner_since(config: &AppConfig) -> Option<String> {
    let file = new_owner_file(config)?;
    let text = match std::fs::read(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::debug!(file = %file.display(), %error, "the new owner mark is not readable; it stands");
            return Some("at an unknown time".to_owned());
        }
    };
    Some(
        serde_json::from_slice::<NewOwnerMark>(&text)
            .map(|record| record.since)
            .unwrap_or_else(|_| "at an unknown time".to_owned()),
    )
}

/// Файл признака копии под памятью базы.
const COPIED_FROM_FILE_NAME: &str = "copied-from.json";

/// Признак копии: содержимое базы пришло из другой базы (`infobase create --from`), и в
/// неё ещё не отправляли. Это вся память о ней
/// (`INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CopiedFrom {
    /// Имя базы-источника в местном слое.
    pub(crate) source: String,
    /// Образ DT, из которого создана база.
    pub(crate) snapshot: PathBuf,
    /// Когда база создана.
    pub(crate) since: chrono::DateTime<chrono::Utc>,
}

/// Признак копии, как его прочла команда. Признак, который не прочесть или не разобрать,
/// всё равно признак: выгрузку не предлагаем, отправка полная.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CopyMark {
    Read(CopiedFrom),
    Unreadable,
}

impl std::fmt::Display for CopyMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(copied) => write!(
                f,
                "the infobase '{}' ({})",
                copied.source,
                copied.since.to_rfc3339()
            ),
            Self::Unreadable => f.write_str("another infobase"),
        }
    }
}

/// Признак копии в каталоге памяти базы — единственное место, где складывается его путь.
fn copy_mark_in(base_memory_dir: &Path) -> PathBuf {
    base_memory_dir.join(COPIED_FROM_FILE_NAME)
}

fn copied_from_file(config: &AppConfig) -> Option<PathBuf> {
    SourceSetsService::new(config)
        .base_memory_dir()
        .map(|dir| copy_mark_in(&dir))
}

/// Записи памяти базы, которые описывают её прежнее содержимое: хеш-память наборов, копии
/// файла версий, журнал поколений и признак нового владельца.
const PREVIOUS_MEMORY: &[&str] = &[
    "hashes",
    "dump-info",
    crate::domain::source_set::GENERATION_FILE_NAME,
    NEW_OWNER_FILE_NAME,
];

/// Память о базе, которую раннер только что создал копией другой базы: прежняя память под
/// её именем стирается — хешей и файла версий у копии нет, — и пишется признак копии с
/// поколением новой базы. Сбой — строка для ответа: база создана, а первая отправка назовёт
/// выходы.
pub(crate) fn remember_copied_base(config: &AppConfig, copied: &CopiedFrom) -> Option<String> {
    let Some(dir) = SourceSetsService::new(config).base_memory_dir() else {
        return Some(
            "the address of the created infobase is not recognized, so the runner keeps no memory of it; the first push names the ways out".to_owned(),
        );
    };
    let file = copy_mark_in(&dir);
    let mut failures: Vec<String> = PREVIOUS_MEMORY
        .iter()
        .filter_map(|name| {
            let entry = dir.join(name);
            let removed = match std::fs::symlink_metadata(&entry) {
                Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&entry),
                Ok(_) => std::fs::remove_file(&entry),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
            removed
                .err()
                .map(|error| format!("'{}': {error}", entry.display()))
        })
        .collect();
    let written = serde_json::to_vec_pretty(copied)
        .map_err(|error| error.to_string())
        .and_then(|text| {
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            crate::support::fs::write_file_atomically(&file, |out| {
                std::io::Write::write_all(out, &text)
            })
            .map_err(|error| format!("'{}': {error}", file.display()))
        });
    if let Err(error) = written {
        failures.push(error);
    }
    (!failures.is_empty()).then(|| {
        format!(
            "the memory of the copied infobase was not written ({}); the first push names the ways out",
            failures.join("; ")
        )
    })
}

/// Снимает признак копии: первая удачная отправка или новая база, собранная из исходников.
pub(crate) fn forget_copied_base(config: &AppConfig) -> Option<String> {
    let file = copied_from_file(config)?;
    match std::fs::remove_file(&file) {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(format!(
            "the mark that the infobase is a copy of another one was not removed from '{}': {error}; the next push loads every source-set in full again and refusals keep offering no pull",
            file.display()
        )),
    }
}

/// Из какой базы скопирована выбранная база, если в неё ещё не отправляли.
pub(crate) fn copied_from(config: &AppConfig) -> Option<CopyMark> {
    read_copy_mark(&copied_from_file(config)?)
}

fn read_copy_mark(file: &Path) -> Option<CopyMark> {
    let text = match std::fs::read(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::debug!(file = %file.display(), %error, "the copy mark is not readable; it stands");
            return Some(CopyMark::Unreadable);
        }
    };
    Some(
        serde_json::from_slice::<CopiedFrom>(&text)
            .map(CopyMark::Read)
            .unwrap_or(CopyMark::Unreadable),
    )
}

/// Положение базы, от которого зависят выходы отказа.
struct Standing {
    /// Копия взяла базу без метки или сменила ушедшего владельца и ещё не отправляла.
    new_owner: Option<String>,
    /// База создана копией другой базы, и в неё ещё не отправляли: из какой.
    copied: Option<CopyMark>,
    /// База в кластере или на автономном сервере: метки у неё нет.
    server: bool,
    /// Общая база (`shared: true`): остальные её владельцы из метки.
    shared_with: Option<Vec<String>>,
}

impl Standing {
    fn of(config: &AppConfig) -> Self {
        Self {
            new_owner: new_owner_since(config),
            copied: copied_from(config),
            server: config.target_kind() != TargetKind::File,
            shared_with: crate::use_cases::infobase_owner::shared_base_owners(config),
        }
    }

    /// Выгрузку предлагают всем, кроме нового владельца до первой отправки. Общей базе
    /// (`shared: true`) — всегда: решение владельца продукта от 06.10.2026 даёт ей оба выхода,
    /// и её меняют другие копии по согласию, а не захват.
    ///
    /// Копии базы до первой отправки выгрузку не предлагают никогда, и общей тоже: её
    /// конфигурация принадлежит ветке источника
    /// (`INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH`).
    fn offers_pull(&self) -> bool {
        self.copied.is_none() && (self.shared_with.is_some() || self.new_owner.is_none())
    }

    /// Почему выгрузка не предложена и кто ещё мог менять базу.
    fn caveats(&self) -> String {
        let mut text = String::new();
        if let Some(source) = &self.copied {
            text.push_str(&format!(
                " The infobase is a copy of {source} and nothing has been pushed into it since: its configuration belongs to the branch of the source, and taking it into this directory would bring that branch here, so only the overwrite is offered."
            ));
        }
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

/// Выход из отказа обмена, который стоит следующим шагом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WayOut {
    /// `pull <SET>`: выгрузка поверх каталога; память запишет ответ о поколении.
    Pull,
    /// `pull <SET> --force`: полная выгрузка, которая и без ответа о поколении пишет память.
    PullForce,
    /// `push [<SET>] --force`: перезапись базы.
    PushForce,
}

impl WayOut {
    /// Следующий шаг отказа. Отказы `no_memory` и `non_fast_forward` строят его только здесь.
    fn step(self, pull_set: &str, push_set: Option<&str>) -> NextStep {
        match self {
            Self::Pull => NextStep::command("pull").for_source_set(pull_set),
            Self::PullForce => NextStep::command("pull")
                .with_key("--force", "")
                .for_source_set(pull_set),
            Self::PushForce => {
                let next = NextStep::command("push").with_key("--force", "");
                match push_set {
                    Some(set) => next.for_source_set(set),
                    None => next,
                }
            }
        }
    }
}

/// Отказ `push` без памяти о базе. Наборы `contexts` — те, что пойдут в базу; каждому
/// нужна своя память: запись журнала поколений или своя хеш-память этой пары. Чужая и
/// нечитаемая хеш-память — отсутствие памяти: она не доказывает, что каталог происходит от
/// этой базы, а полная загрузка (`--full`) её и не читает.
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
    let mut message = format!(
        "cannot push: this working copy has no memory of {target} for source-set {names}, so nothing proves that the sources derive from its state; an empty infobase is not told apart, because its generation depends on the platform version"
    );
    if forgotten
        .iter()
        .any(|set| keeps_other_memory(set, &config.work_path))
    {
        message.push_str(
            "; the memory kept under its name was written for another infobase or source directory, or cannot be read, and is not used",
        );
    }
    // Выгрузка поверх каталога пишет память только через ответ о поколении; инструмент,
    // который его заведомо не даёт, оставил бы копию без памяти, и отказ повторился бы.
    let way_out = match (
        standing.offers_pull(),
        crate::platform::generation::answers_generation(
            config.selected_provider(Operation::Dump),
            config.target_kind(),
        ),
    ) {
        (false, _) => WayOut::PushForce,
        (true, true) => WayOut::Pull,
        (true, false) => WayOut::PullForce,
    };
    match way_out {
        WayOut::Pull | WayOut::PullForce => {
            let pulls = forgotten
                .iter()
                .map(|set| match way_out {
                    WayOut::PullForce => context.advised_pull_force(set.name()),
                    WayOut::Pull | WayOut::PushForce => {
                        context.advised_command(&format!("pull {}", shell_word(set.name())))
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let pull = match way_out {
                WayOut::PullForce => format!(
                    "take its state with a full dump {pulls}, which replaces the source directory and discards its uncommitted changes (the dump over the directory would leave no memory: this tool gives no configuration generation for this infobase)"
                ),
                WayOut::Pull | WayOut::PushForce => format!("see what is there with {pulls}"),
            };
            message.push_str(&format!(
                ". If the infobase holds the right state, {pull}; if the source directory does, run {overwrite}"
            ));
        }
        WayOut::PushForce => message.push_str(&format!(". To load the sources run {overwrite}")),
    }
    message.push('.');
    message.push_str(&standing.caveats());
    Err(UseCaseError::new(UseCaseErrorKind::NoMemory, message)
        .with_next(way_out.step(first.name(), selected_set)))
}

/// Помнит ли рабочая копия базу для набора: своя запись поколения или своя хеш-память
/// этой пары, в том числе пустая — её пишет создание базы раннером. Хеш-память другой пары
/// или нечитаемая отменяет и свою запись поколения: каталог от этой базы она не выводит
/// (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).
fn remembers(set: &SourceSetContext, work_path: &Path) -> bool {
    memory_of(set, work_path) == MemoryState::Remembered
}

/// Что копия помнит о базе для набора — единственное определение памяти: по нему отказывает
/// `push` и отвечает `status`. Своя хеш-память решает первой; без неё — запись журнала
/// поколений, а без неё — признак копии базы. Чужая или нечитаемая хеш-память — не память,
/// даже рядом со своей записью.
pub(crate) fn memory_of(set: &SourceSetContext, work_path: &Path) -> MemoryState {
    if set.storage_identity().is_none() {
        return MemoryState::Unbound;
    }
    match analyzer::snapshot_memory(set, work_path) {
        SnapshotMemory::Own => MemoryState::Remembered,
        SnapshotMemory::Foreign => MemoryState::Foreign,
        SnapshotMemory::Unreadable => MemoryState::Unreadable,
        SnapshotMemory::Nothing => match GenerationLedger::of(set, work_path).map(|l| l.read()) {
            Some(Recorded::Ours(_)) => MemoryState::Remembered,
            Some(Recorded::Foreign { .. }) => MemoryState::Foreign,
            // Копия базы до первой отправки: память — признак копии с поколением новой базы
            // (`INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH`).
            Some(Recorded::Nothing) | None
                if set
                    .base_memory_dir(work_path)
                    .and_then(|dir| read_copy_mark(&copy_mark_in(&dir)))
                    .is_some() =>
            {
                MemoryState::Remembered
            }
            Some(Recorded::Nothing) | None => MemoryState::Missing,
        },
    }
}

/// Лежит ли под именем базы память набора, которая не его: записанная для другой пары или
/// нечитаемая. Её отказ называет, чтобы отсутствие памяти не выглядело чистым листом.
fn keeps_other_memory(set: &SourceSetContext, work_path: &Path) -> bool {
    let foreign_record = GenerationLedger::of(set, work_path)
        .is_some_and(|ledger| matches!(ledger.read(), Recorded::Foreign { .. }));
    foreign_record
        || matches!(
            analyzer::snapshot_memory(set, work_path),
            SnapshotMemory::Foreign | SnapshotMemory::Unreadable
        )
}

/// Запись журнала поколений этой пары «база ↔ каталог»: чужая и неразборчивая — не запись.
pub(crate) fn recorded_generation(
    set: &SourceSetContext,
    work_path: &Path,
) -> Option<GenerationRecord> {
    match GenerationLedger::of(set, work_path)?.read() {
        Recorded::Ours(record) => Some(record),
        Recorded::Nothing | Recorded::Foreign { .. } => None,
    }
}

/// Что покажет сверка поколения перед загрузкой набора — единственное место этого решения:
/// по нему отказывает `push` ([`GenerationGate`]) и отвечает `status --deep`. Ничего не пишет
/// и не отказывает. Токены сравниваются только внутри одного инструмента
/// (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`).
///
/// `answer` — инструмент и токен, которыми ответила база; `None` — ответа нет.
pub(crate) fn predict(
    record: Option<&GenerationRecord>,
    answer: Option<(Provider, &str)>,
) -> GenerationVerdict {
    let Some(record) = record else {
        return GenerationVerdict::NoRecord;
    };
    let Some((tool, token)) = answer else {
        return GenerationVerdict::NoAnswer;
    };
    match record.compare(tool, token) {
        GenerationComparison::Unchanged => GenerationVerdict::Unchanged,
        GenerationComparison::Changed => GenerationVerdict::MovedAhead,
        GenerationComparison::NoAnswer => GenerationVerdict::OtherTool,
    }
}

/// Сколько набора загружено: от этого зависит, доказано ли совпадение каталога и базы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadExtent {
    /// Набор загружен целиком.
    Whole,
    /// Загружены изменившиеся файлы.
    Partial,
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
    mode: PushMode,
    /// Сверки, сделанные заранее, до первой загрузки команды: по имени набора.
    checked: RefCell<HashMap<String, BeforeLoad>>,
    /// Что неудачная загрузка оставила для ответа о поколении.
    failed_load: RefCell<Option<String>>,
}

impl<'a> GenerationGate<'a> {
    pub(crate) fn new(
        context: &'a ExecutionContext,
        config: &'a AppConfig,
        mode: PushMode,
    ) -> Self {
        Self {
            context,
            config,
            mode,
            checked: RefCell::new(HashMap::new()),
            failed_load: RefCell::new(None),
        }
    }

    /// Сверка заранее всех наборов, которые команда загрузит, до первой загрузки
    /// (`INV.USE-CASES.EVERY-SET-IS-CHECKED-BEFORE-THE-FIRST-LOAD`): отказ по набору, идущему не
    /// первым, не должен приходить после того, как наборы перед ним уже легли в базу. Чтения
    /// те же, что сделала бы сверка перед загрузкой каждого набора, — только раньше: их ответы
    /// [`Self::before_load`] берёт, а не спрашивает снова. Одному набору сверка заранее не
    /// нужна: его сверит загрузка. Отказ — с именем набора.
    pub(crate) fn check_early(
        &self,
        sets: &[&SourceSetContext],
        tool: Provider,
        mut read: impl FnMut(&SourceSetContext) -> Result<Option<String>, AppError>,
    ) -> Result<(), (String, AppError)> {
        if sets.len() < 2 {
            return Ok(());
        }
        for set in sets {
            let before = self
                .compare(set, tool, || read(set))
                .map_err(|error| (set.name().to_owned(), error))?;
            self.checked
                .borrow_mut()
                .insert(set.name().to_owned(), before);
        }
        Ok(())
    }

    /// Перед загрузкой набора: поколение базы, прочитанное тем же инструментом, что записал
    /// прошлое, отличается от записанного — отказ `non_fast_forward` до загрузки. Без записи
    /// того же инструмента поколение не читается: сравнивать не с чем. Набор, сверенный
    /// заранее ([`Self::check_early`]), не спрашивается снова.
    pub(crate) fn before_load(
        &self,
        set: &SourceSetContext,
        tool: Provider,
        read: impl FnOnce() -> Result<Option<String>, AppError>,
    ) -> Result<BeforeLoad, AppError> {
        if let Some(before) = self.checked.borrow_mut().remove(set.name()) {
            return Ok(before);
        }
        self.compare(set, tool, read)
    }

    fn compare(
        &self,
        set: &SourceSetContext,
        tool: Provider,
        read: impl FnOnce() -> Result<Option<String>, AppError>,
    ) -> Result<BeforeLoad, AppError> {
        // После отмены поколение не спрашивают: загрузку остановит её безопасная точка.
        if self.mode == PushMode::Force
            || crate::use_cases::interruption::pending_interruption_error(
                self.context,
                "the configuration generation",
            )
            .is_some()
        {
            return Ok(BeforeLoad::Unchecked);
        }
        let Some(record) = self.record_of(set, tool) else {
            return Ok(BeforeLoad::Unchecked);
        };
        let Some(token) = read()? else {
            return Ok(BeforeLoad::Unchecked);
        };
        match predict(Some(&record), Some((tool, &token))) {
            GenerationVerdict::Unchanged => Ok(BeforeLoad::Matched),
            GenerationVerdict::MovedAhead => Err(self.moved_ahead(set.name(), &token, &record)),
            GenerationVerdict::OtherTool
            | GenerationVerdict::NoRecord
            | GenerationVerdict::NoAnswer => Ok(BeforeLoad::Unchecked),
        }
    }

    /// Запись набора, сделанная тем же инструментом.
    fn record_of(&self, set: &SourceSetContext, tool: Provider) -> Option<GenerationRecord> {
        recorded_generation(set, &self.config.work_path).filter(|record| record.tool == tool)
    }

    fn moved_ahead(&self, set: &str, base: &str, record: &GenerationRecord) -> AppError {
        let standing = Standing::of(self.config);
        let target = self.config.v8_connection().describe_target();
        let push_force = self
            .context
            .advised_command(&format!("push {} --force", shell_word(set)));
        let local = &record.token;
        let recorded_at = &record.recorded_at;
        let mut message = match record.after {
            GenerationAfter::FailedBuild => format!(
                "cannot push source-set '{set}': the configuration generation of {target} is {base}, not {local} that was recorded before the last push of this working copy failed ({recorded_at}); that failed push or another working copy changed the infobase, and the runner cannot tell which"
            ),
            GenerationAfter::Build | GenerationAfter::Dump => format!(
                "cannot push source-set '{set}': {target} moved ahead since the last exchange of this working copy — its configuration generation is {base}, the one recorded after the last {} ({recorded_at}) is {local}",
                record.after
            ),
        };
        let pull = self
            .context
            .advised_command(&format!("pull {}", shell_word(set)));
        let way_out = if standing.offers_pull() {
            WayOut::Pull
        } else {
            WayOut::PushForce
        };
        match (way_out, record.after) {
            // Расхождение после своей неудачной загрузки чаще всего её же след: естественный
            // выход — повторить загрузку перезаписью; выгрузка — если базу правил кто-то ещё.
            (WayOut::Pull, GenerationAfter::FailedBuild) => message.push_str(&format!(
                "; if the change is that failed push, load the directory again with {push_force}; if someone else changed the infobase, take their changes first with {pull}"
            )),
            (WayOut::Pull, GenerationAfter::Build | GenerationAfter::Dump) => message.push_str(
                &format!("; take its changes first with {pull}, or overwrite them with {push_force}"),
            ),
            (WayOut::PullForce | WayOut::PushForce, _) => {
                message.push_str(&format!("; to overwrite them run {push_force}"));
            }
        }
        message.push('.');
        message.push_str(&standing.caveats());
        AppError::Refused(Box::new(
            UseCaseError::new(UseCaseErrorKind::NonFastForward, message)
                .with_next(way_out.step(set, Some(set)))
                .with_generations(Generations {
                    base: base.to_owned(),
                    local: local.clone(),
                }),
        ))
    }

    /// После удачной загрузки набора: поколение записывается с инструментом. Без ответа
    /// запись набора стирается — прежний токен описывает уже не ту базу, и следующая
    /// отправка не должна принять свою же загрузку за чужую правку; стёртую запись ответ
    /// называет. Загрузка уже прошла, поэтому сбой записи — предупреждение, а не отказ.
    #[must_use]
    pub(crate) fn after_load(
        &self,
        set: &SourceSetContext,
        tool: Provider,
        token: Option<&str>,
    ) -> Option<String> {
        let ledger = GenerationLedger::of(set, &self.config.work_path)?;
        let name = set.name();
        match token {
            Some(token) => match ledger.record(tool, token, GenerationAfter::Build) {
                Ok(()) => None,
                Err(error) => Some(match ledger.forget() {
                    Ok(_) => format!(
                        "the configuration generation of source-set '{name}' was not recorded: {error}; its previous record is erased, so the next push does not check whether the infobase moved ahead"
                    ),
                    Err(forget) => format!(
                        "the configuration generation of source-set '{name}' was not recorded: {error}; its previous record was not erased either ({forget}), so the next push may take this load for a change made elsewhere"
                    ),
                }),
            },
            None => match ledger.forget() {
                Ok(false) => None,
                Ok(true) => Some(format!(
                    "the configuration generation of source-set '{name}' is not known after the load: its previous record is erased, so the next push does not check whether the infobase moved ahead"
                )),
                Err(error) => Some(format!(
                    "the configuration generation of source-set '{name}' is not known after the load, and its previous record was not erased: {error}; the next push may take this load for a change made elsewhere"
                )),
            },
        }
    }

    /// После неудачной загрузки набора: поколение не записывается — неизвестно, что
    /// неудачная загрузка успела сделать с базой. Запись того же инструмента помечается как
    /// сделанная перед неудачной загрузкой: если поколение с ней разойдётся, следующая
    /// отправка откажет `non_fast_forward` и скажет, что базу изменила либо эта загрузка,
    /// либо другая копия, а не выдаст своё за чужое; совпадёт — загрузка базу не тронула.
    /// Строку для шага неудачной загрузки отдаёт [`Self::failed_load_note`].
    pub(crate) fn after_failed_load(&self, set: &SourceSetContext, tool: Provider) {
        let name = set.name();
        let not_recorded = format!(
            "the configuration generation of source-set '{name}' is not recorded after the failed load"
        );
        let marked = GenerationLedger::of(set, &self.config.work_path).and_then(|ledger| {
            self.record_of(set, tool)
                .map(|record| ledger.record(tool, &record.token, GenerationAfter::FailedBuild))
        });
        let note = match marked {
            None => not_recorded,
            Some(Ok(())) => format!(
                "{not_recorded}: if the infobase no longer has the generation recorded before it, the next push is refused until the infobase is pulled or overwritten"
            ),
            Some(Err(error)) => format!(
                "{not_recorded}, and the record made before it was not marked as preceding a failed load: {error}"
            ),
        };
        *self.failed_load.borrow_mut() = Some(note);
    }

    /// Что ответ неудачной загрузки говорит о поколении: строка [`Self::after_failed_load`].
    pub(crate) fn failed_load_note(&self) -> Vec<String> {
        self.failed_load.borrow_mut().take().into_iter().collect()
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
        extent: LoadExtent,
        before: &BeforeLoad,
        token: Option<&str>,
        dump_and_reread: impl FnOnce() -> Option<Result<Option<String>, AppError>>,
    ) -> Option<String> {
        let file = set.path().join(VERSION_FILE_NAME);
        let proven = extent == LoadExtent::Whole || *before == BeforeLoad::Matched;
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
            Ok(_) | Err(_) => Some(match std::fs::remove_file(&file) {
                Ok(()) => format!(
                    "{VERSION_FILE_NAME} is missing from source-set '{}' and was not restored: the configuration generation did not confirm that the infobase stayed unchanged while it was dumped; the next pull runs full",
                    set.name()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => format!(
                    "{VERSION_FILE_NAME} is missing from source-set '{}' and was not restored: the configuration generation did not confirm that the infobase stayed unchanged; the next pull runs full",
                    set.name()
                ),
                // Оставшийся файл объявил бы каталог совпадающим с базой: его называют, чтобы
                // его убрали до следующей выгрузки по изменившемуся.
                Err(error) => format!(
                    "{VERSION_FILE_NAME} was dumped alone into source-set '{}' while the configuration generation did not confirm that the infobase stayed unchanged, and it could not be removed: {error}; delete '{}' before the next pull, otherwise it claims that the directory matches the infobase",
                    set.name(),
                    file.display()
                ),
            }),
        }
    }
}

/// Пропуск выгрузки по изменившемуся: поколение до неё совпало с записанным тем же
/// инструментом — в базе нечего брать с прошлого обмена. Строка — ответ «всё актуально»;
/// `None` — выгружать (`INV.USE-CASES.AN-UNCHANGED-GENERATION-IS-NOT-DUMPED`).
pub(crate) fn unchanged_since_the_record(
    set: &SourceSetContext,
    work_path: &Path,
    tool: Provider,
    token: &str,
) -> Option<String> {
    let Recorded::Ours(record) = GenerationLedger::of(set, work_path)?.read() else {
        return None;
    };
    (record.compare(tool, token) == GenerationComparison::Unchanged).then(|| {
        format!(
            "configuration generation {token} is unchanged since the last {} ({}); nothing to dump",
            record.after, record.recorded_at
        )
    })
}

/// Поколение у выгрузки: до и после. Совпало — запись после выгрузки. Изменилось — базу
/// правили во время выгрузки: ответ это называет, а записывается поколение, которое было
/// до выгрузки, — и следующая отправка, увидев другое, откажет `non_fast_forward`, даже
/// если памяти о базе до этой выгрузки не было. Без ответа до или после запись не меняется.
/// Выгрузка уже прошла, поэтому сбой записи — строка для ответа, а не отказ.
pub(crate) fn record_after_dump(
    set: &SourceSetContext,
    work_path: &Path,
    tool: Provider,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<String> {
    let (Some(before), Some(after)) = (before, after) else {
        return None;
    };
    let ledger = GenerationLedger::of(set, work_path)?;
    if let Err(error) = ledger.record(tool, before, GenerationAfter::Dump) {
        return Some(format!(
            "the configuration generation of source-set '{}' was not recorded: {error}",
            set.name()
        ));
    }
    if before == after {
        return None;
    }
    Some(format!(
        "the infobase was changed while source-set '{}' was being dumped: its configuration generation was {before} before the dump and {after} after it; the generation from before the dump is remembered, so the next push is refused as non-fast-forward — pull again",
        set.name()
    ))
}

/// Память о наборе, из которого раннер собирает созданную базу: дерево набора в формате
/// Конфигуратора, а у формата EDT ещё и дерево его исходников EDT. Дерево исходников
/// снимается до сборки: правка, сделанная во время неё, остаётся изменением для первой
/// отправки.
pub(crate) struct AssembledMemory {
    source_set: String,
    designer: analyzer::FullSnapshot,
    edt: Option<analyzer::FullSnapshot>,
}

/// Дерево исходников EDT набора, снятое до их перевода в XML.
pub(crate) struct EdtSourceMemory(analyzer::FullSnapshot);

impl EdtSourceMemory {
    /// Снимает дерево исходников EDT набора `source_set` по его контексту памяти.
    pub(crate) fn prepare(
        config: &AppConfig,
        source_set: &crate::config::model::SourceSetConfig,
    ) -> Result<Self, AppError> {
        let contexts = SourceSetsService::new(config).edt_contexts();
        snapshot_of(&contexts, &source_set.name).map(Self)
    }
}

impl AssembledMemory {
    /// Снимает дерево набора `source_set` в формате Конфигуратора по его контексту памяти: у
    /// формата Конфигуратора — сами исходники, у формата EDT — их перевод в XML.
    pub(crate) fn prepare(
        config: &AppConfig,
        source_set: &crate::config::model::SourceSetConfig,
    ) -> Result<Self, AppError> {
        let contexts = SourceSetsService::new(config).designer_contexts();
        Ok(Self {
            source_set: source_set.name.clone(),
            designer: snapshot_of(&contexts, &source_set.name)?,
            edt: None,
        })
    }

    /// Дерево исходников EDT, из которых переведён набор.
    pub(crate) fn with_edt_source(mut self, edt: EdtSourceMemory) -> Self {
        self.edt = Some(edt.0);
        self
    }
}

fn snapshot_of(
    contexts: &[SourceSetContext],
    source_set: &str,
) -> Result<analyzer::FullSnapshot, AppError> {
    let context = contexts
        .iter()
        .find(|context| context.name() == source_set)
        .ok_or_else(|| {
            AppError::Runtime(format!(
                "missing change-detection context for source-set '{source_set}'"
            ))
        })?;
    analyzer::prepare_full_snapshot(context, context.path())
        .map_err(|error| AppError::Runtime(error.to_string()))
}

/// Память о базе, которую раннер только что создал: у набора, из которого база собрана
/// (`assembled`), — его дерево, снятое до сборки, у каждого другого набора, который в неё
/// пойдёт, — пустая хеш-память этой пары, и первая отправка грузит его целиком без отказа
/// первого знакомства. Признак нового владельца снимается: база своя с рождения. Сбой —
/// строка для ответа: база создана, а первая отправка без памяти откажет и назовёт выходы.
pub(crate) fn remember_created_base(
    config: &AppConfig,
    assembled: Option<&AssembledMemory>,
) -> Option<String> {
    let failures: Vec<String> = SourceSetsService::new(config)
        .designer_contexts()
        .iter()
        .filter(|set| set.storage_identity().is_some())
        .filter_map(|set| {
            let committed = match assembled {
                Some(memory) if memory.source_set == set.name() => {
                    analyzer::commit_full_snapshot(set, &config.work_path, &memory.designer)
                }
                _ => analyzer::commit_empty_snapshot(set, &config.work_path),
            };
            committed
                .err()
                .map(|error| format!("source-set '{}': {error}", set.name()))
        })
        .chain(assembled.and_then(|memory| remember_edt_source(config, memory)))
        .chain(forget_new_owner(config))
        .chain(forget_copied_base(config))
        .collect();
    (!failures.is_empty()).then(|| {
        format!(
            "the memory of the created infobase was not written ({}); the first push names the ways out",
            failures.join("; ")
        )
    })
}

/// Память об исходниках EDT собранного набора: первая отправка не переводит его заново.
fn remember_edt_source(config: &AppConfig, memory: &AssembledMemory) -> Option<String> {
    let edt = memory.edt.as_ref()?;
    let contexts = SourceSetsService::new(config).edt_contexts();
    let context = contexts
        .iter()
        .find(|context| context.name() == memory.source_set)?;
    analyzer::commit_full_snapshot(context, &config.work_path, edt)
        .err()
        .map(|error| format!("EDT sources of source-set '{}': {error}", memory.source_set))
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

        assert_eq!(remember_created_base(&config, None), None);

        require(&config).expect("the created base is remembered");
        assert_eq!(new_owner_since(&config), None);
    }

    /// Набор, из которого раннер собрал созданную базу, память помнит его деревом, снятым
    /// до сборки: неизменный набор первая отправка не грузит, а правка, сделанная после
    /// снятия, остаётся изменением.
    #[test]
    fn a_created_base_remembers_the_set_it_was_assembled_from() {
        use crate::change_detection::analyzer::{analyze_context, AnalysisOutcome};
        let root = tempfile::tempdir().expect("tempdir");
        let config = project(root.path());
        let main = config.source_sets[0].clone();
        std::fs::write(
            main.root_in(&config.base_path).join("Configuration.xml"),
            "<Configuration/>",
        )
        .expect("source");
        let memory = AssembledMemory::prepare(&config, &main).expect("assembled memory");

        assert_eq!(remember_created_base(&config, Some(&memory)), None);

        let contexts = SourceSetsService::new(&config).designer_contexts();
        let analysis = analyze_context(&contexts[0], &config.work_path);
        assert!(
            matches!(analysis.outcome, Ok(AnalysisOutcome::NoChanges)),
            "{:?}",
            analysis.outcome
        );
        std::fs::write(
            main.root_in(&config.base_path).join("Module.bsl"),
            "changed",
        )
        .expect("edit");
        let analysis = analyze_context(&contexts[0], &config.work_path);
        assert!(
            matches!(analysis.outcome, Ok(AnalysisOutcome::Changes { .. })),
            "{:?}",
            analysis.outcome
        );
    }

    /// Признак нового владельца, который не прочесть, стоит: выгрузку отказ не предлагает.
    #[test]
    fn an_unreadable_new_owner_mark_stands() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = project(root.path());
        let file = new_owner_file(&config).expect("remembered base");
        // Каталог на месте файла: прочесть его как файл нельзя и под root.
        std::fs::create_dir_all(&file).expect("unreadable mark");

        assert!(new_owner_since(&config).is_some());
        assert!(!Standing::of(&config).offers_pull());
        let refused = require(&config).expect_err("no memory");
        assert_eq!(
            refused.next().map(|next| next.command.as_str()),
            Some("push")
        );
    }

    /// Признак копии — память каждого набора: отказа первого знакомства нет. Признак, который
    /// не прочесть, стоит так же, и выгрузку не предлагают; созданная из исходников база его
    /// снимает.
    #[test]
    fn a_copy_mark_is_memory_and_offers_no_pull() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = project(root.path());
        let file = copied_from_file(&config).expect("remembered base");
        assert_eq!(require(&config).map_err(|_| ()), Err(()));
        let copied = CopiedFrom {
            source: "upstream".to_owned(),
            snapshot: root.path().join("work/copies/upstream.dt"),
            since: chrono::Utc::now(),
        };

        assert_eq!(remember_copied_base(&config, &copied), None);
        require(&config).expect("the copy mark is memory");
        assert!(!Standing::of(&config).offers_pull());
        assert!(Standing::of(&config).caveats().contains("'upstream'"));

        std::fs::remove_file(&file).expect("mark");
        std::fs::create_dir_all(&file).expect("unreadable mark");
        require(&config).expect("an unreadable mark stands");
        assert!(!Standing::of(&config).offers_pull());

        std::fs::remove_dir(&file).expect("mark");
        assert_eq!(remember_copied_base(&config, &copied), None);
        assert_eq!(remember_created_base(&config, None), None);
        assert!(!file.exists(), "a base assembled anew is no copy");
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
        remember_created_base(&config, None);
        assert!(!holds_only_new_owner_marks(
            &root.path().join("work/infobases")
        ));
    }
}
