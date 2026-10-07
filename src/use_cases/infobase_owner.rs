//! Метка владельца файловой базы: какая рабочая копия держит базу.
//!
//! Базу, с которой ведут разработку, держит одна рабочая копия
//! (`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`). Кто её держит, записано
//! в метке рядом с каталогом базы, снаружи него: копия каталога базы метку не уносит. Форма
//! метки закреплена `CTR.USE-CASES.INFOBASE-OWNER-MARKER`.
//!
//! Проверку зовёт только граница команды (`use_cases::transport`), сразу после замка базы:
//! команда записи на базе другой живой копии отказывает `InfobaseHeld`, если делить базу не
//! согласны эта копия или кто-то из владельцев (`shared: true` в местном слое,
//! `INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY`); прошедшая проверку команда
//! записи на базе из местного слоя записывает свою копию в метку со своим согласием. Команда
//! чтения метку только читает; превью читает её без замка и ничего не пишет.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::loader::{load_declared_infobases, ConfigLoadError};
use crate::config::model::{AppConfig, InfobaseConfig};
use crate::domain::next_step::NextStep;
use crate::platform::connection::V8Connection;
use crate::support::fs::{read_optional, write_file_atomically};
use crate::support::machine::{host_name, machine_id};
use crate::support::path::{nearest_existing_canonical_path, stable_path_identity};
use crate::use_cases::infobase_lock::BaseAccess;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Версия формы метки, которую пишет и понимает этот раннер.
pub const OWNER_MARKER_VERSION: u32 = 1;

/// Файл, в котором лежит порождённая форма метки.
#[allow(dead_code, reason = "читает сторож свежести артефакта")]
pub const OWNER_MARKER_SCHEMA_PATH: &str = "docs/schemas/infobase-owner-marker.schema.json";

/// Хвост имени файла метки: `.<имя каталога базы>.v8-runner.owners.json`.
const OWNER_MARKER_SUFFIX: &str = ".v8-runner.owners.json";

/// Метка владельца файловой базы.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "v8-runner infobase owner marker",
    description = "Кто держит файловую базу: метка лежит рядом с каталогом базы как `.<имя каталога>.v8-runner.owners.json`."
)]
pub struct OwnerMarker {
    /// Версия формы. Метку другой версии раннер не переписывает.
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    /// Рабочие копии, которые держат базу.
    pub owners: Vec<OwnerRecord>,
}

/// Рабочая копия, которая держит базу.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OwnerRecord {
    /// Машина копии: SHA-256 от `v8-runner/owner/` и идентификатора машины, который
    /// переживает смену имени хоста (`machine-id` у Linux, аппаратный UUID у macOS,
    /// `MachineGuid` у Windows; без него — `host:<имя хоста>`), шестнадцатеричной строкой.
    /// Сам идентификатор в метку не попадает: `machine-id(5)` просит его не показывать.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub machine: String,
    /// Имя хоста на момент записи — для людей; машину называет `machine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Канонический абсолютный каталог проекта копии: там лежит её местный слой.
    pub project: PathBuf,
    /// Согласие копии делить базу.
    pub shared: bool,
    /// Когда копия записана в метку.
    #[schemars(with = "String", extend("format" = "date-time"))]
    pub since: DateTime<Utc>,
}

/// Эта рабочая копия: машина и каталог проекта.
#[derive(Debug, Clone)]
pub(crate) struct ThisCopy {
    /// Хеш идентификатора машины, как его пишет метка; `None`, если машина не называет ни
    /// идентификатора, ни имени хоста.
    machine: Option<String>,
    host: Option<String>,
    project: PathBuf,
}

impl ThisCopy {
    /// Копия, которой принадлежит проект `config`, на этой машине.
    fn of(config: &AppConfig) -> Self {
        let host = host_name();
        let identity = machine_id().or_else(|| host.as_ref().map(|host| format!("host:{host}")));
        Self::on(identity.as_deref(), host, &config.base_path)
    }

    /// Копия с идентификатором машины `identity` в открытом виде; в метку идёт его хеш.
    fn on(identity: Option<&str>, host: Option<String>, project: &Path) -> Self {
        Self {
            machine: identity.map(machine_hash),
            host,
            project: canonical(project),
        }
    }

    fn is(&self, record: &OwnerRecord) -> bool {
        self.is_on_the_machine_of(record) && same_path(&record.project, &self.project)
    }

    fn is_on_the_machine_of(&self, record: &OwnerRecord) -> bool {
        self.machine.as_deref() == Some(record.machine.as_str())
    }
}

/// Хеш идентификатора машины для метки: идентификатор в открытом виде не хранится.
fn machine_hash(identity: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("v8-runner/owner/{identity}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Чего команда ждёт от проверки: прогон записывает свою копию, превью только называет отказ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerCheck {
    Run,
    Preview,
}

/// Проверяет, чья база, и записывает эту копию в метку, если должна.
///
/// Возвращает то, что ответ команды говорит сверх своего: взятие базы без метки, смену
/// ушедшего владельца, нечитаемую метку у команды чтения. Отказ — `InfobaseHeld` на базе
/// другой живой копии, если делить её согласны не все: эта копия и каждый живой владелец;
/// `Runtime` — когда метку не прочитать, не понять или не записать.
/// Метку пишет только прогон команды записи, и только под замком базы: его держит
/// вызывающий.
pub(crate) fn check_infobase_owner(
    config: &AppConfig,
    command_name: &str,
    access: BaseAccess,
    check: OwnerCheck,
) -> Result<Vec<String>, UseCaseError> {
    check_as(&ThisCopy::of(config), config, command_name, access, check)
}

fn check_as(
    this: &ThisCopy,
    config: &AppConfig,
    command_name: &str,
    access: BaseAccess,
    check: OwnerCheck,
) -> Result<Vec<String>, UseCaseError> {
    let writes = match access {
        BaseAccess::Untouched => return Ok(Vec::new()),
        BaseAccess::Reads => false,
        BaseAccess::Writes => true,
    };
    let Some(base_dir) = config.v8_connection().file_infobase_dir(&config.base_path) else {
        return Ok(Vec::new());
    };
    let Some(marker_path) = owner_marker_path(&base_dir) else {
        return Ok(Vec::new());
    };
    let beside = marker_path.parent().unwrap_or(&base_dir).to_path_buf();
    if !writes {
        if !beside.exists() {
            return Ok(Vec::new());
        }
        return Ok(match read_marker(&marker_path) {
            Ok(_) => Vec::new(),
            Err(error) => vec![format!(
                "{}; {command_name} reads the infobase '{}' without knowing who holds it",
                error.describe(&beside),
                base_dir.display()
            )],
        });
    }

    let marker = read_marker(&marker_path).map_err(|error| {
        UseCaseError::new(
            UseCaseErrorKind::Runtime,
            format!(
                "cannot start {command_name}: {}; a command that writes the infobase '{}' does not run without knowing who holds it",
                error.describe(&beside),
                base_dir.display()
            ),
        )
    })?;
    let owners: Vec<(OwnerRecord, Standing)> = marker
        .map(|marker| marker.owners)
        .unwrap_or_default()
        .into_iter()
        .map(|owner| {
            let standing = standing(this, &owner, &base_dir);
            (owner, standing)
        })
        .collect();
    let nobody_held_it = owners.is_empty();
    let this_consent = ThisConsent::of(config, &base_dir);
    let shares = this_consent == ThisConsent::Shares;
    // Метку пишет только прогон команды записи на базе, названной в местном слое.
    let records = check == OwnerCheck::Run && config.infobase_name.is_some();

    let alive: Vec<(&OwnerRecord, &Alive)> = owners
        .iter()
        .filter_map(|(owner, standing)| match standing {
            Standing::Alive(why) => Some((owner, why)),
            Standing::This | Standing::Gone(_) => None,
        })
        .collect();
    let recorded_consent = owners
        .iter()
        .find_map(|(owner, standing)| matches!(standing, Standing::This).then_some(owner.shared));

    if !alive.is_empty() && (!shares || alive.iter().any(|(owner, why)| !why.consents(owner))) {
        // Копия, уже записанная в метке, сообщает своё согласие и при отказе: копии других
        // машин видят отзыв только так. Ушедших отказ не сменяет.
        let unrecorded = (records && recorded_consent.is_some_and(|recorded| recorded != shares))
            .then(|| {
                let reported = OwnerMarker {
                    version: OWNER_MARKER_VERSION,
                    owners: rewritten(&owners, shares, Departed::Stay),
                };
                write_marker(&marker_path, &reported).err()
            })
            .flatten();
        return Err(HeldRefusal {
            command_name,
            base_dir: &base_dir,
            marker_path: &marker_path,
            this: this_consent,
            alive,
            unrecorded,
        }
        .into_error());
    }
    // Превью ничего не берёт, а строка соединения подчиняется владельцу, но им не
    // становится — даже на базе без метки или с ушедшим владельцем.
    if !records {
        return Ok(Vec::new());
    }
    let gone: Vec<(&OwnerRecord, &Gone)> = owners
        .iter()
        .filter_map(|(owner, standing)| match standing {
            Standing::Gone(why) => Some((owner, why)),
            Standing::This | Standing::Alive(_) => None,
        })
        .collect();
    if recorded_consent == Some(shares) && gone.is_empty() {
        return Ok(Vec::new());
    }
    let Some(machine) = this.machine.clone() else {
        return Ok(vec![format!(
            "this machine gives neither an identifier nor a host name, so the working copy '{}' is not recorded as the owner of the infobase '{}'",
            this.project.display(),
            base_dir.display()
        )]);
    };

    let mut notes = Vec::new();
    for (owner, why) in &gone {
        notes.push(format!(
            "the working copy '{}' no longer holds the infobase '{}': {}; this working copy '{}' holds it now (owner marker '{}')",
            owner.project.display(),
            base_dir.display(),
            why.describe(),
            this.project.display(),
            marker_path.display()
        ));
    }
    if nobody_held_it && base_dir.is_dir() {
        notes.push(format!(
            "the infobase '{}' had no owner and is now held by this working copy '{}' (owner marker '{}')",
            base_dir.display(),
            this.project.display(),
            marker_path.display()
        ));
    }
    // Взявшая базу копия до первой удачной отправки выгрузку не предлагает: признак пишется
    // в её память о базе раньше метки, чтобы метка без признака не появилась.
    if !notes.is_empty() {
        crate::use_cases::exchange_guard::remember_new_owner(config).map_err(|error| {
            UseCaseError::new(
                UseCaseErrorKind::Runtime,
                format!(
                    "cannot start {command_name}: the mark that this working copy took the infobase '{}' over cannot be written: {error}",
                    base_dir.display()
                ),
            )
        })?;
    }
    let mut kept = rewritten(&owners, shares, Departed::Leave);
    if recorded_consent.is_none() {
        kept.push(OwnerRecord {
            machine,
            host: this.host.clone(),
            project: this.project.clone(),
            shared: shares,
            since: Utc::now(),
        });
    }
    let marker = OwnerMarker {
        version: OWNER_MARKER_VERSION,
        owners: kept,
    };
    write_marker(&marker_path, &marker).map_err(|error| {
        UseCaseError::new(
            UseCaseErrorKind::Runtime,
            format!(
                "cannot start {command_name}: the owner marker cannot be written next to '{}': {error}; a command that writes the infobase '{}' does not run without recording who holds it",
                beside.display(),
                base_dir.display()
            ),
        )
    })?;
    Ok(notes)
}

/// Остальные владельцы общей базы — для отказов обмена с ней
/// (`INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH`). `None` — эта
/// копия базу не делит (или это не файловая база); иначе — копии из метки, кроме этой:
/// каталог проекта и хост. Метку, которую не прочитать, отказ обмена не называет.
pub(crate) fn shared_base_owners(config: &AppConfig) -> Option<Vec<String>> {
    let base_dir = config
        .v8_connection()
        .file_infobase_dir(&config.base_path)?;
    if ThisConsent::of(config, &base_dir) != ThisConsent::Shares {
        return None;
    }
    let this = ThisCopy::of(config);
    let owners = owner_marker_path(&base_dir)
        .and_then(|path| read_marker(&path).ok().flatten())
        .map(|marker| marker.owners)
        .unwrap_or_default();
    Some(
        owners
            .iter()
            .filter(|owner| !this.is(owner))
            .map(|owner| match &owner.host {
                Some(host) => format!("'{}' on '{host}'", owner.project.display()),
                None => format!("'{}'", owner.project.display()),
            })
            .collect(),
    )
}

/// Копии, которые держат выбранную файловую базу, по её метке — для `status --deep`
/// (`INV.CLI.STATUS-DEEP-NAMES-THE-OWNING-COPY`). Только читает: ни замка, ни записи в метку
/// (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`). `None` — база не файловая.
pub(crate) fn holders(config: &AppConfig) -> Option<crate::domain::status::HoldersStatus> {
    use crate::domain::status::{HolderStatus, HoldersStatus};
    let base_dir = config
        .v8_connection()
        .file_infobase_dir(&config.base_path)?;
    let marker = owner_marker_path(&base_dir)?;
    let beside = marker.parent().unwrap_or(&base_dir).to_path_buf();
    let this = ThisCopy::of(config);
    Some(match read_marker(&marker) {
        Ok(read) => HoldersStatus {
            owners: Some(
                read.map(|marker| marker.owners)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|owner| HolderStatus {
                        this_copy: this.is(&owner),
                        project: owner.project,
                        host: owner.host,
                        shared: owner.shared,
                        since: owner.since,
                    })
                    .collect(),
            ),
            reason: None,
            marker,
        },
        Err(error) => HoldersStatus {
            owners: None,
            reason: Some(error.describe(&beside)),
            marker,
        },
    })
}

/// Согласие этой копии делить базу: из её местного слоя в момент команды.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThisConsent {
    Shares,
    DoesNotShare,
    /// База пришла строкой соединения в `--infobase`: согласия у неё нет, она базу не делит,
    /// а только подчиняется владельцу.
    ConnectionString,
}

impl ThisConsent {
    fn of(config: &AppConfig, base_dir: &Path) -> Self {
        if config.infobase_name.is_none() {
            return Self::ConnectionString;
        }
        // Выбранная секция объявляет базу всегда; остальные секции той же базы тоже
        // должны согласиться — так же, как у копии-владельца.
        let sections = std::iter::once(&config.infobase).chain(config.infobases.values());
        match consent_of(sections, &config.base_path, base_dir) {
            Some(true) => Self::Shares,
            Some(false) | None => Self::DoesNotShare,
        }
    }
}

/// Согласие копии делить файловую базу `base_dir` по секциям её местного слоя, пути — от её
/// каталога `project`. `None` — ни одна секция базу не объявляет; иначе копия согласна, только
/// если `shared: true` стоит у каждой секции, которая базу объявляет.
fn consent_of<'a>(
    sections: impl IntoIterator<Item = &'a InfobaseConfig>,
    project: &Path,
    base_dir: &Path,
) -> Option<bool> {
    let mut declaring = sections
        .into_iter()
        .filter(|infobase| {
            V8Connection::from_connection_string(&infobase.connection)
                .file_infobase_dir(project)
                .is_some_and(|dir| same_path(&dir, base_dir))
        })
        .peekable();
    declaring.peek()?;
    Some(declaring.all(|infobase| infobase.shared))
}

/// Что делать с ушедшими владельцами, когда метка переписывается.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Departed {
    /// Команда записи прошла и сменяет их.
    Leave,
    /// Команда отказала: метку она меняет только согласием этой копии.
    Stay,
}

/// Записи метки после команды: у этой копии — её нынешнее согласие, живые остаются,
/// ушедшие — по `departed`.
fn rewritten(
    owners: &[(OwnerRecord, Standing)],
    shares: bool,
    departed: Departed,
) -> Vec<OwnerRecord> {
    owners
        .iter()
        .filter_map(|(owner, standing)| match standing {
            Standing::This => Some(OwnerRecord {
                shared: shares,
                ..owner.clone()
            }),
            Standing::Alive(_) => Some(owner.clone()),
            Standing::Gone(_) => (departed == Departed::Stay).then(|| owner.clone()),
        })
        .collect()
}

/// Где копия-владелец по отношению к этой команде.
enum Standing {
    /// Эта самая копия.
    This,
    /// Живая другая копия: базу она держит.
    Alive(Alive),
    /// Ушедшая копия этой машины: её сменяет команда записи.
    Gone(Gone),
}

enum Alive {
    /// Каталог копии на этой машине есть и объявляет базу; `shared` — согласие делить её,
    /// прочитанное из её местного слоя сейчас.
    Declares { shared: bool },
    /// Местный слой копии не прочитать: она считается живой и несогласной. Причина — без
    /// текста чужих файлов: в нём бывают пароли.
    Unreadable(String),
    /// Копия с другой машины: проверить её отсюда нельзя. `maybe_this_machine` — тот же
    /// каталог проекта и то же имя хоста, что у этой копии: возможно, это эта машина, у
    /// которой сменился идентификатор.
    Remote { maybe_this_machine: bool },
}

impl Alive {
    /// Согласна ли копия делить базу. Копия этой машины отвечает своим местным слоем,
    /// нечитаемый слой — несогласием, копия другой машины — своей записью в метке.
    fn consents(&self, record: &OwnerRecord) -> bool {
        match self {
            Self::Declares { shared } => *shared,
            Self::Unreadable(_) => false,
            Self::Remote { .. } => record.shared,
        }
    }
}

enum Gone {
    DirectoryIsGone,
    NoLongerDeclares,
}

impl Gone {
    fn describe(&self) -> &'static str {
        match self {
            Self::DirectoryIsGone => "its directory is gone",
            Self::NoLongerDeclares => "its local layer no longer declares the infobase",
        }
    }
}

fn standing(this: &ThisCopy, owner: &OwnerRecord, base_dir: &Path) -> Standing {
    if this.is(owner) {
        return Standing::This;
    }
    if !this.is_on_the_machine_of(owner) {
        let maybe_this_machine = owner.host.is_some()
            && owner.host == this.host
            && same_path(&owner.project, &this.project);
        return Standing::Alive(Alive::Remote { maybe_this_machine });
    }
    // Ушедшим владельца делает только ответ «нет такого каталога»: каталог, который не
    // прочитать, ещё может объявлять базу.
    match std::fs::metadata(&owner.project) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Standing::Gone(Gone::DirectoryIsGone),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Standing::Gone(Gone::DirectoryIsGone)
        }
        Err(error) => {
            return Standing::Alive(Alive::Unreadable(format!(
                "its directory cannot be read: {}",
                error.kind()
            )))
        }
    }
    // Объявленный путь разрешается от каталога владельца: у проекта, скопированного
    // целиком, `File=build/ib` указывает на его собственную базу, а не на копию.
    match load_declared_infobases(&owner.project) {
        // Текст ошибки разбора цитирует файл, а в местном слое лежат пароли: причина
        // называется без него.
        Err(ConfigLoadError::ReadError(error)) => Standing::Alive(Alive::Unreadable(format!(
            "its v8project.yaml or v8project.local.yaml cannot be read: {}",
            error.kind()
        ))),
        Err(_) => Standing::Alive(Alive::Unreadable(
            "its v8project.yaml or v8project.local.yaml cannot be parsed".to_owned(),
        )),
        Ok(declared) => match consent_of(declared.values(), &owner.project, base_dir) {
            Some(shared) => Standing::Alive(Alive::Declares { shared }),
            None => Standing::Gone(Gone::NoLongerDeclares),
        },
    }
}

/// Отказ на базе другой копии: кто её держит и согласен ли делить её, как освободить базу,
/// где метка и какие выходы есть у этой копии. Следующий шаг — первый и безопасный: своя
/// чистая база. Выгрузку отказ не предлагает: базу держит другая копия, и выгрузка увела бы эту
/// копию в её ветку.
struct HeldRefusal<'a> {
    command_name: &'a str,
    base_dir: &'a Path,
    marker_path: &'a Path,
    this: ThisConsent,
    alive: Vec<(&'a OwnerRecord, &'a Alive)>,
    /// Почему согласие этой копии не записалось в метку при отказе.
    unrecorded: Option<std::io::Error>,
}

impl HeldRefusal<'_> {
    fn into_error(self) -> UseCaseError {
        let Self {
            command_name,
            base_dir,
            marker_path,
            this,
            alive,
            unrecorded,
        } = self;
        let holders = alive
        .iter()
        .map(|(owner, why)| {
            let sharing = if why.consents(owner) {
                "which shares it"
            } else {
                "which does not share it"
            };
            match why {
                Alive::Declares { .. } => format!(
                    "the working copy '{}' on this machine, {sharing}",
                    owner.project.display()
                ),
                Alive::Unreadable(reason) => format!(
                    "the working copy '{}' on this machine, whose local layer cannot be read ({reason}) and which therefore counts as holding it and not sharing it",
                    owner.project.display()
                ),
                Alive::Remote { .. } => format!(
                    "the working copy '{}' on machine '{}', {sharing} by its record in the owner marker",
                    owner.project.display(),
                    owner.host.as_deref().unwrap_or("with an unknown host name")
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
        let this_copy = match this {
        ThisConsent::Shares => "this working copy shares it",
        ThisConsent::DoesNotShare => "this working copy does not share it",
        ThisConsent::ConnectionString => {
            "a connection string does not share an infobase — pass its name (declared in v8project.local.yaml with shared: true)"
        }
    };
        let release = alive
        .iter()
        .map(|(owner, why)| match why {
            Alive::Declares { .. } | Alive::Unreadable(_) => format!(
                "remove the infobase from v8project.local.yaml of '{}' or remove that working copy",
                owner.project.display()
            ),
            Alive::Remote {
                maybe_this_machine: false,
            } => format!(
                "on another machine only by hand: delete its record of '{}' from the owner marker",
                owner.project.display()
            ),
            Alive::Remote {
                maybe_this_machine: true,
            } => format!(
                "the record of '{}' names this project and this host name with another machine identifier — perhaps it is this machine whose identifier changed: then delete that record from the owner marker",
                owner.project.display()
            ),
        })
        .collect::<Vec<_>>()
        .join("; ");
        let unrecorded = unrecorded
            .map(|error| {
                format!(
                ". The consent of this working copy cannot be recorded in the owner marker: {error}"
            )
            })
            .unwrap_or_default();
        UseCaseError::new(
        UseCaseErrorKind::InfobaseHeld,
        format!(
            "cannot start {command_name}: the infobase '{}' is held by {holders}; {this_copy}. A command that writes a development infobase runs only in the working copy that holds it, or on a shared infobase when every working copy that holds it and this one share it, and repeating it does not help. \
             Ways out for this working copy: its own clean infobase — `v8-runner init --infobase <connection string>` points infobases.origin at an infobase of its own (the previous section stays as upstream), then `v8-runner infobase create`; \
             a copy of the infobase with its data — the same `init --infobase <connection string>`, then `v8-runner infobase create --from upstream`, which snapshots this infobase while it is free and creates the new one from the image; \
             a shared infobase — `shared: true` at the infobase in v8project.local.yaml of every working copy that holds it and of this one: a working copy on another machine reports its consent through the owner marker at its next write command. \
             To free the infobase: {release}. Owner marker: '{}'{unrecorded}",
            base_dir.display(),
            marker_path.display()
        ),
    )
    .with_next(NextStep::command("infobase create"))
    }
}

/// Почему метку не прочитать.
#[derive(Debug)]
enum MarkerReadError {
    Io(std::io::Error),
    Malformed(String),
    UnknownVersion(serde_json::Value),
}

impl MarkerReadError {
    fn describe(&self, beside: &Path) -> String {
        let beside = beside.display();
        match self {
            Self::Io(error) => format!("the owner marker next to '{beside}' cannot be read: {error}"),
            Self::Malformed(error) => {
                format!("the owner marker next to '{beside}' cannot be understood: {error}")
            }
            Self::UnknownVersion(version) => format!(
                "the owner marker next to '{beside}' has version {version}, and this runner knows version {OWNER_MARKER_VERSION}; a marker of another version is not rewritten — use a runner that knows it, or remove the marker by hand"
            ),
        }
    }
}

/// Метка рядом с базой: `None`, если её ещё нет. Версия сверяется раньше формы: незнакомая
/// версия называется как версия, а не как непонятная форма.
fn read_marker(path: &Path) -> Result<Option<OwnerMarker>, MarkerReadError> {
    let Some(raw) = read_optional(path).map_err(MarkerReadError::Io)? else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|error| MarkerReadError::Malformed(error.to_string()))?;
    match value.get("version") {
        Some(version) if version.as_u64() == Some(u64::from(OWNER_MARKER_VERSION)) => {}
        Some(version) => return Err(MarkerReadError::UnknownVersion(version.clone())),
        None => {
            return Err(MarkerReadError::Malformed(
                "the marker names no version".to_owned(),
            ))
        }
    }
    let marker: OwnerMarker = serde_json::from_value(value)
        .map_err(|error| MarkerReadError::Malformed(error.to_string()))?;
    // Неабсолютный каталог владельца раннер разрешил бы от своего каталога и спросил бы не
    // того владельца: такую метку он не понимает.
    if let Some(owner) = marker
        .owners
        .iter()
        .find(|owner| !owner.project.is_absolute())
    {
        return Err(MarkerReadError::Malformed(format!(
            "the owner record names the project '{}', which is not an absolute path",
            owner.project.display()
        )));
    }
    Ok(Some(marker))
}

/// Пишет метку заменой файла целиком: читатель без замка видит прежнюю метку или новую.
fn write_marker(path: &Path, marker: &OwnerMarker) -> std::io::Result<()> {
    let mut encoded = serde_json::to_vec_pretty(marker).map_err(std::io::Error::other)?;
    encoded.push(b'\n');
    write_file_atomically(path, |file| file.write_all(&encoded))
}

/// Файл метки рядом с каталогом базы.
fn owner_marker_path(base_dir: &Path) -> Option<PathBuf> {
    let parent = base_dir.parent()?;
    let name = base_dir.file_name()?;
    let mut marker_name = std::ffi::OsString::from(".");
    marker_name.push(name);
    marker_name.push(OWNER_MARKER_SUFFIX);
    Some(parent.join(marker_name))
}

fn canonical(path: &Path) -> PathBuf {
    nearest_existing_canonical_path(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Пути сравниваются канонически и так же, как у замков: на macOS и Windows без учёта
/// регистра.
fn same_path(left: &Path, right: &Path) -> bool {
    stable_path_identity(&canonical(left)) == stable_path_identity(&canonical(right))
}

/// Форма метки, порождённая из типов.
#[allow(dead_code, reason = "читает сторож свежести артефакта")]
pub fn generated_owner_marker_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(OwnerMarker);
    let mut value = crate::command_data::generated_schema(schema, "infobase-owner-marker");
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "$id".to_owned(),
            serde_json::Value::String(format!(
                "{}/infobase-owner-marker.schema.json",
                crate::command_data::REPOSITORY_RAW_SCHEMA_ROOT
            )),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{
        check_as, generated_owner_marker_schema, owner_marker_path, read_marker, OwnerCheck,
        OwnerMarker, ThisCopy, OWNER_MARKER_SCHEMA_PATH,
    };
    use crate::config::model::{AppConfig, InfobaseConfig, SourceFormat, TestsConfig, ToolsConfig};
    use crate::use_cases::infobase_lock::BaseAccess;
    use crate::use_cases::result::UseCaseErrorKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn project(root: &Path, base: &Path) -> AppConfig {
        shared_project(root, base, false)
    }

    /// Копия в `root`, чей местный слой объявляет `base` с согласием `shared` делить её.
    fn shared_project(root: &Path, base: &Path, shared: bool) -> AppConfig {
        fs::create_dir_all(root).expect("project");
        fs::write(
            root.join("v8project.local.yaml"),
            format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n    shared: {shared}\n",
                base.display()
            ),
        )
        .expect("local layer");
        let mut infobase = InfobaseConfig::file(format!("File={}", base.display()));
        infobase.shared = shared;
        AppConfig {
            base_path: fs::canonicalize(root).expect("canonical project"),
            work_path: root.join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase,
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            source_sets: Vec::new(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn base(dir: &Path) -> PathBuf {
        let base = dir.join("shared").join("ib");
        fs::create_dir_all(&base).expect("base");
        fs::canonicalize(base).expect("canonical base")
    }

    fn marker(base: &Path) -> OwnerMarker {
        read_marker(&owner_marker_path(base).expect("marker path"))
            .expect("readable")
            .expect("present")
    }

    fn on(machine: &str, host: &str, config: &AppConfig) -> ThisCopy {
        ThisCopy::on(Some(machine), Some(host.to_owned()), &config.base_path)
    }

    /// Машину называет её идентификатор, а не имя хоста: после смены имени копия узнаёт
    /// себя, а копию этой машины, чей каталог исчез, сменяет как ушедшую — а не держит как
    /// копию с другой машины.
    #[test]
    fn a_host_rename_keeps_the_machine() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let first = project(&dir.path().join("first"), &base);
        let second = project(&dir.path().join("second"), &base);

        check_as(
            &on("machine-a", "old-name", &first),
            &first,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("the first copy takes the base");
        let recorded = marker(&base);

        let notes = check_as(
            &on("machine-a", "new-name", &first),
            &first,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("the renamed machine is the same machine");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(marker(&base), recorded, "the owner is not written again");

        let refused = check_as(
            &on("machine-a", "new-name", &second),
            &second,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect_err("a live copy of the renamed machine holds the base");
        assert_eq!(refused.kind(), UseCaseErrorKind::InfobaseHeld);

        fs::remove_dir_all(dir.path().join("first")).expect("remove the first copy");
        let notes = check_as(
            &on("machine-a", "new-name", &second),
            &second,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("a gone copy of this machine is replaced");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(marker(&base).owners.len(), 1);
        assert_eq!(marker(&base).owners[0].project, second.base_path);
        assert_eq!(marker(&base).owners[0].host.as_deref(), Some("new-name"));
    }

    /// Пока замок базы держит другая команда, проверка до метки не доходит: граница
    /// отказывает раньше, и метку не пишет никто, кроме держателя замка.
    #[test]
    fn a_base_whose_lock_is_busy_keeps_its_marker() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let first = project(&dir.path().join("first"), &base);
        let second = project(&dir.path().join("second"), &base);
        let _held = crate::use_cases::infobase_lock::acquire_infobase_lock(
            &first,
            "push",
            BaseAccess::Writes,
        )
        .expect("the first command holds the base");

        let refused = crate::use_cases::transport::dispatch_with_workspace_lock(
            &second,
            crate::use_cases::context::CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || (),
        )
        .expect_err("the base lock is busy");

        assert_eq!(refused.error.kind(), UseCaseErrorKind::InfobaseBusy);
        assert!(!owner_marker_path(&base).expect("marker path").exists());
    }

    /// Превью и команда чтения метку не заводят; команда записи со строкой соединения —
    /// тоже.
    #[test]
    fn only_a_run_of_a_write_on_a_declared_base_records_a_copy() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        let this = on("machine-a", "host", &config);
        let marker_path = owner_marker_path(&base).expect("marker path");

        check_as(
            &this,
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Preview,
        )
        .expect("preview");
        check_as(
            &this,
            &config,
            "infobase.dump",
            BaseAccess::Reads,
            OwnerCheck::Run,
        )
        .expect("read");
        let mut ad_hoc = config.clone();
        ad_hoc.infobase_name = None;
        check_as(&this, &ad_hoc, "push", BaseAccess::Writes, OwnerCheck::Run).expect("ad hoc");
        assert!(!marker_path.exists());

        check_as(&this, &config, "push", BaseAccess::Writes, OwnerCheck::Run).expect("run");
        assert!(marker_path.exists());
    }

    /// Порождённая форма метки совпадает с закреплённым артефактом.
    #[test]
    fn generated_owner_marker_schema_is_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(OWNER_MARKER_SCHEMA_PATH);
        let generated = crate::command_data::schema_json_pretty(&generated_owner_marker_schema());
        if std::env::var_os("UPDATE_OWNER_MARKER_SCHEMA").is_some() {
            fs::write(&path, &generated).expect("write owner marker schema");
        }
        let actual = fs::read_to_string(&path).expect("owner marker schema artifact");
        assert_eq!(
            actual, generated,
            "{OWNER_MARKER_SCHEMA_PATH} is stale; rerun UPDATE_OWNER_MARKER_SCHEMA=1 cargo test generated_owner_marker_schema_is_current"
        );
    }

    /// Метку, которую пишет раннер, принимает её закреплённая форма.
    #[test]
    fn a_written_marker_passes_its_schema() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");
        let written: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(owner_marker_path(&base).expect("marker path")).expect("marker"),
        )
        .expect("json");

        let schema = generated_owner_marker_schema();
        let validator = jsonschema::validator_for(&schema).expect("schema");
        assert!(validator.is_valid(&written), "{written}");
    }

    /// Метку, которую не записать, команда записи не обходит: отказ называет каталог и
    /// причину, а метки не появляется. Каталог рядом с базой закрыт на запись; под root права
    /// не действуют, и тогда проверке не на чем стоять.
    #[cfg(unix)]
    #[test]
    fn a_marker_that_cannot_be_written_stops_a_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        let marker_path = owner_marker_path(&base).expect("marker path");
        let beside = base.parent().expect("parent").to_path_buf();
        fs::set_permissions(&beside, fs::Permissions::from_mode(0o555)).expect("chmod");
        let probe = beside.join("probe");
        if fs::write(&probe, "").is_ok() {
            fs::remove_file(&probe).expect("remove probe");
            eprintln!("skipped: permissions do not apply to this user");
            return;
        }

        let refused = check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        );
        fs::set_permissions(&beside, fs::Permissions::from_mode(0o755)).expect("chmod back");

        let refused = refused.expect_err("the marker cannot be written");
        assert_eq!(refused.kind(), UseCaseErrorKind::Runtime);
        assert!(
            refused.message().contains(&beside.display().to_string()),
            "{refused}"
        );
        assert!(refused.message().contains("cannot be written"), "{refused}");
        assert!(!marker_path.exists());
    }

    /// Метка хранит хеш идентификатора машины, а не сам идентификатор.
    #[test]
    fn the_marker_keeps_the_machine_hashed() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        check_as(
            &on("4c2f8e0a9b7d41d6a1f3c5e7d9b2a4c6", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");

        let text =
            fs::read_to_string(owner_marker_path(&base).expect("marker path")).expect("marker");
        assert!(!text.contains("4c2f8e0a9b7d41d6a1f3c5e7d9b2a4c6"), "{text}");
        let machine = &marker(&base).owners[0].machine;
        assert_eq!(
            machine,
            &super::machine_hash("4c2f8e0a9b7d41d6a1f3c5e7d9b2a4c6")
        );
        assert_eq!(machine.len(), 64);
        assert!(machine
            .chars()
            .all(|ch| matches!(ch, '0'..='9' | 'a'..='f')));
    }

    /// Запись этого проекта с этим именем хоста, но с другой машиной раннер не сменяет —
    /// это может быть и другая машина с тем же путём, — но отказ подсказывает, что это,
    /// возможно, эта машина со сменившимся идентификатором.
    #[test]
    fn a_record_of_this_project_and_host_on_another_machine_is_named_as_maybe_this_one() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        check_as(
            &on("old-machine", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");

        let refused = check_as(
            &on("new-machine", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect_err("another machine holds the base");

        assert_eq!(refused.kind(), UseCaseErrorKind::InfobaseHeld);
        assert!(
            refused
                .message()
                .contains("perhaps it is this machine whose identifier changed"),
            "{refused}"
        );
    }

    /// Машина без идентификатора и без имени хоста подчиняется владельцу, но в метку себя не
    /// записывает и говорит об этом.
    #[test]
    fn a_machine_without_an_identity_is_not_recorded() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);

        let notes = check_as(
            &ThisCopy::on(None, None, &config.base_path),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");

        assert!(
            notes.iter().any(|note| note.contains("not recorded")),
            "{notes:?}"
        );
        assert!(!owner_marker_path(&base).expect("marker path").exists());
    }

    /// Метку, которая называет каталог владельца не абсолютным путём, раннер не понимает:
    /// команда записи отказывает, а метку не трогает.
    #[test]
    fn a_marker_with_a_relative_project_is_not_understood() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        let marker_path = owner_marker_path(&base).expect("marker path");
        let text = format!(
            "{{\"version\":1,\"owners\":[{{\"machine\":\"{}\",\"project\":\"copy\",\"shared\":false,\"since\":\"2026-10-01T00:00:00Z\"}}]}}",
            super::machine_hash("machine-a")
        );
        fs::write(&marker_path, &text).expect("marker");

        let refused = check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect_err("a relative project is not understood");

        assert_eq!(refused.kind(), UseCaseErrorKind::Runtime);
        assert!(
            refused.message().contains("not an absolute path"),
            "{refused}"
        );
        assert_eq!(fs::read_to_string(&marker_path).expect("marker"), text);
    }

    fn consents(base: &Path) -> Vec<(PathBuf, bool)> {
        marker(base)
            .owners
            .into_iter()
            .map(|owner| (owner.project, owner.shared))
            .collect()
    }

    /// Копия другой машины сообщает согласие меткой: отозвав его, она получает отказ, и её
    /// запись в метке обновляется под замком; после этого отказывает и эта машина и
    /// называет её. До её команды отзыв отсюда не виден.
    #[test]
    fn a_remote_copy_that_withdrew_consent_stops_this_machine_after_its_next_write() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let here = shared_project(&dir.path().join("here"), &base, true);
        let there = shared_project(&dir.path().join("there"), &base, true);
        let this_machine = on("machine-a", "here-host", &here);
        let other_machine = on("machine-b", "there-host", &there);

        check_as(
            &this_machine,
            &here,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("this machine takes the base");
        check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("the other machine shares it");
        assert_eq!(
            consents(&base),
            [
                (here.base_path.clone(), true),
                (there.base_path.clone(), true)
            ]
        );

        let there = shared_project(&dir.path().join("there"), &base, false);
        check_as(
            &this_machine,
            &here,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("the withdrawal is not seen before the other copy reports it");

        let refused = check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect_err("the copy that withdrew is refused");
        assert_eq!(refused.kind(), UseCaseErrorKind::InfobaseHeld);
        assert!(
            refused
                .message()
                .contains(&here.base_path.display().to_string()),
            "{refused}"
        );
        assert_eq!(
            consents(&base),
            [
                (here.base_path.clone(), true),
                (there.base_path.clone(), false)
            ],
            "the refused copy reports its withdrawal through the marker"
        );

        let refused = check_as(
            &this_machine,
            &here,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect_err("this machine sees the withdrawal");
        assert_eq!(refused.kind(), UseCaseErrorKind::InfobaseHeld);
        assert!(
            refused.message().contains(&format!(
                "'{}' on machine 'there-host', which does not share it",
                there.base_path.display()
            )),
            "{refused}"
        );
    }

    /// Превью и команда чтения согласие в метку не пишут: его сообщает только прогон команды
    /// записи под замком базы.
    #[test]
    fn only_a_run_of_a_write_reports_consent() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let here = shared_project(&dir.path().join("here"), &base, true);
        let there = shared_project(&dir.path().join("there"), &base, true);
        let this_machine = on("machine-a", "here-host", &here);
        let other_machine = on("machine-b", "there-host", &there);
        check_as(
            &this_machine,
            &here,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");
        check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");
        let recorded = marker(&base);

        let there = shared_project(&dir.path().join("there"), &base, false);
        check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Preview,
        )
        .expect_err("the preview names the refusal");
        check_as(
            &other_machine,
            &there,
            "infobase.dump",
            BaseAccess::Reads,
            OwnerCheck::Run,
        )
        .expect("a read passes");

        assert_eq!(marker(&base), recorded);
    }

    /// Единственный владелец, сменивший согласие, обновляет свою запись: копия с другой
    /// машины узнает о нём из метки.
    #[test]
    fn a_sole_owner_records_its_changed_consent() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = shared_project(&dir.path().join("copy"), &base, false);
        let this = on("machine-a", "host", &config);
        check_as(&this, &config, "push", BaseAccess::Writes, OwnerCheck::Run).expect("run");
        assert_eq!(consents(&base), [(config.base_path.clone(), false)]);

        let config = shared_project(&dir.path().join("copy"), &base, true);
        let notes =
            check_as(&this, &config, "push", BaseAccess::Writes, OwnerCheck::Run).expect("run");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(consents(&base), [(config.base_path.clone(), true)]);
    }

    /// Согласие, которое не записать в метку при отказе, отказ не подменяет: ответ остаётся
    /// `InfobaseHeld` и говорит, что согласие не записано. Каталог рядом с базой закрыт на
    /// запись; под root права не действуют, и тогда проверке не на чем стоять.
    #[cfg(unix)]
    #[test]
    fn a_consent_that_cannot_be_recorded_keeps_the_held_refusal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let here = shared_project(&dir.path().join("here"), &base, true);
        let there = shared_project(&dir.path().join("there"), &base, true);
        let other_machine = on("machine-b", "there-host", &there);
        check_as(
            &on("machine-a", "here-host", &here),
            &here,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");
        check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");
        let recorded = marker(&base);
        let there = shared_project(&dir.path().join("there"), &base, false);
        let beside = base.parent().expect("parent").to_path_buf();
        fs::set_permissions(&beside, fs::Permissions::from_mode(0o555)).expect("chmod");
        let probe = beside.join("probe");
        if fs::write(&probe, "").is_ok() {
            fs::remove_file(&probe).expect("remove probe");
            fs::set_permissions(&beside, fs::Permissions::from_mode(0o755)).expect("chmod back");
            eprintln!("skipped: permissions do not apply to this user");
            return;
        }

        let refused = check_as(
            &other_machine,
            &there,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        );
        fs::set_permissions(&beside, fs::Permissions::from_mode(0o755)).expect("chmod back");

        let refused = refused.expect_err("the copy that withdrew is refused");
        assert_eq!(refused.kind(), UseCaseErrorKind::InfobaseHeld);
        assert!(
            refused.message().contains("cannot be recorded"),
            "{refused}"
        );
        assert_eq!(marker(&base), recorded);
    }
}
