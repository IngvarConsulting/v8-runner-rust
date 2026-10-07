//! Метка владельца файловой базы: какая рабочая копия держит базу.
//!
//! Базу, с которой ведут разработку, держит одна рабочая копия
//! (`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`). Кто её держит, записано
//! в метке рядом с каталогом базы, снаружи него: копия каталога базы метку не уносит. Форма
//! метки закреплена `CTR.USE-CASES.INFOBASE-OWNER-MARKER`.
//!
//! Проверку зовёт только граница команды (`use_cases::transport`), сразу после замка базы.
//! Команда записи на базе другой живой копии не отказывает: она идёт, а ответ предупреждает,
//! чью базу она меняет и как завести свою
//! (`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`); метку такая
//! команда не трогает. Команда записи на базе из местного слоя, у которой нет живого владельца,
//! кроме этой копии, записывает свою копию в метку. Команда чтения метку только читает; превью
//! читает её без замка и ничего не пишет.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::loader::{load_declared_infobases, ConfigLoadError};
use crate::config::model::{AppConfig, InfobaseConfig};
use crate::platform::connection::V8Connection;
use crate::support::fs::{read_optional, write_file_atomically};
use crate::support::machine::{host_name, machine_id};
use crate::support::path::{nearest_existing_canonical_path, stable_path_identity};
use crate::use_cases::infobase_lock::BaseAccess;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Версия формы метки, которую пишет этот раннер.
pub const OWNER_MARKER_VERSION: u32 = 2;

/// Прежняя версия формы, которую раннер читает: её записи несли согласие `shared` делить
/// базу. Общих баз больше нет, и согласие при чтении отбрасывается; метку этой версии раннер
/// переписывает новой, когда записывает в неё свою копию.
const LEGACY_OWNER_MARKER_VERSION: u32 = 1;

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
    /// Версия формы. Метку незнакомой версии раннер не переписывает; метку версии 1 он
    /// читает без её поля `shared`.
    #[schemars(range(min = 2, max = 2))]
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

/// Чего команда ждёт от проверки: прогон записывает свою копию, превью только называет то,
/// что сказал бы прогон.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerCheck {
    Run,
    Preview,
}

/// Проверяет, чья база, и записывает эту копию в метку, если должна.
///
/// Возвращает то, что ответ команды говорит сверх своего: запись в базу другой живой копии,
/// взятие базы без метки, смену ушедшего владельца, нечитаемую метку у команды чтения.
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

    let alive: Vec<(&OwnerRecord, &Alive)> = owners
        .iter()
        .filter_map(|(owner, standing)| match standing {
            Standing::Alive(why) => Some((owner, why)),
            Standing::This | Standing::Gone(_) => None,
        })
        .collect();
    // База другой живой копии: команда идёт с предупреждением, а метку не трогает — ни
    // прогон, ни превью. Владельцем остаётся прежняя копия, и её следующая команда сама
    // увидит, что база ушла вперёд её памяти.
    if !alive.is_empty() {
        return Ok(vec![another_copy_warning(
            command_name,
            &base_dir,
            &marker_path,
            &alive,
        )]);
    }
    // Превью ничего не берёт, а строка соединения владельцем не становится — даже на базе
    // без метки или с ушедшим владельцем.
    let records = check == OwnerCheck::Run && config.infobase_name.is_some();
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
    let recorded = owners
        .iter()
        .any(|(_, standing)| matches!(standing, Standing::This));
    if recorded && gone.is_empty() {
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
    // Ушедших сменяет эта копия; живых здесь уже нет.
    let mut kept: Vec<OwnerRecord> = owners
        .iter()
        .filter(|(_, standing)| matches!(standing, Standing::This))
        .map(|(owner, _)| owner.clone())
        .collect();
    if !recorded {
        kept.push(OwnerRecord {
            machine,
            host: this.host.clone(),
            project: this.project.clone(),
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

/// Объявляет ли какая-нибудь секция местного слоя копии файловую базу `base_dir`; пути — от
/// каталога копии `project`.
fn declares<'a>(
    sections: impl IntoIterator<Item = &'a InfobaseConfig>,
    project: &Path,
    base_dir: &Path,
) -> bool {
    sections.into_iter().any(|infobase| {
        V8Connection::from_connection_string(&infobase.connection)
            .file_infobase_dir(project)
            .is_some_and(|dir| same_path(&dir, base_dir))
    })
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
    /// Каталог копии на этой машине есть и объявляет базу.
    Declares,
    /// Местный слой копии не прочитать: она считается живой. Причина — без текста чужих
    /// файлов: в нём бывают пароли.
    Unreadable(String),
    /// Копия с другой машины: проверить её отсюда нельзя. `maybe_this_machine` — тот же
    /// каталог проекта и то же имя хоста, что у этой копии: возможно, это эта машина, у
    /// которой сменился идентификатор.
    Remote { maybe_this_machine: bool },
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
        Ok(declared) if declares(declared.values(), &owner.project, base_dir) => {
            Standing::Alive(Alive::Declares)
        }
        Ok(_) => Standing::Gone(Gone::NoLongerDeclares),
    }
}

/// Предупреждение команды записи на базе другой копии: чья база, что команда её меняет,
/// где метка и как завести свою базу. Метку команда не меняет: владельцем остаётся прежняя
/// копия.
fn another_copy_warning(
    command_name: &str,
    base_dir: &Path,
    marker_path: &Path,
    alive: &[(&OwnerRecord, &Alive)],
) -> String {
    let holders = alive
        .iter()
        .map(|(owner, why)| match why {
            Alive::Declares => format!(
                "the working copy '{}' on this machine",
                owner.project.display()
            ),
            Alive::Unreadable(reason) => format!(
                "the working copy '{}' on this machine, whose local layer cannot be read ({reason}) and which therefore counts as holding it",
                owner.project.display()
            ),
            Alive::Remote { maybe_this_machine } => {
                let maybe = if *maybe_this_machine {
                    " (its record names this project and this host name with another machine identifier — perhaps it is this machine whose identifier changed: then delete that record from the owner marker)"
                } else {
                    ""
                };
                format!(
                    "the working copy '{}' on machine '{}'{maybe}",
                    owner.project.display(),
                    owner.host.as_deref().unwrap_or("with an unknown host name")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "{command_name} writes the infobase '{}' of another working copy: it is held by {holders}. \
         The command changes that working copy's infobase, and its owner stays as it is in the owner marker '{}'. \
         Ways to an infobase of this working copy's own: a copy of this infobase with its data — `v8-runner init --infobase <connection string>` points infobases.origin at an infobase of its own (the previous section stays as upstream), then `v8-runner infobase create --from upstream`; \
         an infobase deployed from a reference image — the same `init --infobase <connection string>`, then `v8-runner infobase restore --input <reference>.dt --create`; \
         a bare infobase built from the sources — the same `init --infobase <connection string>`, then `v8-runner infobase create`",
        base_dir.display(),
        marker_path.display()
    )
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
                "the owner marker next to '{beside}' has version {version}, and this runner knows version {OWNER_MARKER_VERSION} (and reads version {LEGACY_OWNER_MARKER_VERSION}); a marker of another version is not rewritten — use a runner that knows it, or remove the marker by hand"
            ),
        }
    }
}

/// Метка рядом с базой: `None`, если её ещё нет. Версия сверяется раньше формы: незнакомая
/// версия называется как версия, а не как непонятная форма. Метка прежней версии читается
/// без согласия `shared` её записей.
fn read_marker(path: &Path) -> Result<Option<OwnerMarker>, MarkerReadError> {
    let Some(raw) = read_optional(path).map_err(MarkerReadError::Io)? else {
        return Ok(None);
    };
    let mut value: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|error| MarkerReadError::Malformed(error.to_string()))?;
    let Some(version) = value.get("version").cloned() else {
        return Err(MarkerReadError::Malformed(
            "the marker names no version".to_owned(),
        ));
    };
    match version.as_u64() {
        Some(known) if known == u64::from(OWNER_MARKER_VERSION) => {}
        Some(legacy) if legacy == u64::from(LEGACY_OWNER_MARKER_VERSION) => {
            forget_legacy_consent(&mut value)?;
        }
        _ => return Err(MarkerReadError::UnknownVersion(version)),
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

/// Метка версии 1 в форме нынешней версии: согласие `shared` у записей отбрасывается — общих
/// баз больше нет, — остальная форма та же. Согласие версия 1 требовала логическим значением;
/// иное значение — непонятная метка, как и раньше.
fn forget_legacy_consent(value: &mut serde_json::Value) -> Result<(), MarkerReadError> {
    if let Some(owners) = value
        .get_mut("owners")
        .and_then(serde_json::Value::as_array_mut)
    {
        for owner in owners
            .iter_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            match owner.remove("shared") {
                Some(serde_json::Value::Bool(_)) => {}
                Some(other) => {
                    return Err(MarkerReadError::Malformed(format!(
                        "the owner record of version {LEGACY_OWNER_MARKER_VERSION} gives `shared` as {other}, not as true or false"
                    )))
                }
                None => {
                    return Err(MarkerReadError::Malformed(format!(
                        "the owner record of version {LEGACY_OWNER_MARKER_VERSION} names no `shared`"
                    )))
                }
            }
        }
    }
    value["version"] = serde_json::Value::from(OWNER_MARKER_VERSION);
    Ok(())
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

    /// Копия в `root`, чей местный слой объявляет `base`.
    fn project(root: &Path, base: &Path) -> AppConfig {
        fs::create_dir_all(root).expect("project");
        fs::write(
            root.join("v8project.local.yaml"),
            format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n",
                base.display()
            ),
        )
        .expect("local layer");
        let infobase = InfobaseConfig::file(format!("File={}", base.display()));
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
        let base = dir.join("bases").join("ib");
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

        let warned = check_as(
            &on("machine-a", "new-name", &second),
            &second,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("a write on a base of a live copy runs");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(
            warned[0].contains(&first.base_path.display().to_string()),
            "{warned:?}"
        );
        assert_eq!(marker(&base), recorded, "the owner stays");

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

        let recorded = marker(&base);

        let warned = check_as(
            &on("new-machine", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("a write on a base of another machine runs");

        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(
            warned[0].contains("perhaps it is this machine whose identifier changed"),
            "{warned:?}"
        );
        assert_eq!(
            marker(&base),
            recorded,
            "the record of the other machine stays"
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
            "{{\"version\":2,\"owners\":[{{\"machine\":\"{}\",\"project\":\"copy\",\"since\":\"2026-10-01T00:00:00Z\"}}]}}",
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

    /// Метка версии 1 читается без согласия `shared`: копия, уже записанная в ней, метку не
    /// переписывает; запись другой живой копии в ней идёт с предупреждением и метку не трогает;
    /// смена ушедшего владельца переписывает её формой нынешней версии.
    #[test]
    fn a_marker_of_version_one_is_read_without_its_consent() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let config = project(&dir.path().join("copy"), &base);
        let marker_path = owner_marker_path(&base).expect("marker path");
        let legacy = |owners: &[(&str, &Path)]| {
            let owners = owners
                .iter()
                .map(|(machine, project)| {
                    serde_json::json!({
                        "machine": super::machine_hash(machine),
                        "host": "host",
                        "project": project,
                        "shared": true,
                        "since": "2026-10-01T00:00:00Z"
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({"version": 1, "owners": owners}).to_string()
        };

        let own = legacy(&[("machine-a", &config.base_path)]);
        fs::write(&marker_path, &own).expect("marker");
        let notes = check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("this copy holds the base");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(fs::read_to_string(&marker_path).expect("marker"), own);

        let remote = legacy(&[("machine-b", Path::new("/srv/elsewhere"))]);
        fs::write(&marker_path, &remote).expect("marker");
        let warned = check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("a write on a base of another copy runs");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(warned[0].contains("/srv/elsewhere"), "{warned:?}");
        assert_eq!(fs::read_to_string(&marker_path).expect("marker"), remote);

        let gone = dir.path().join("gone");
        fs::write(&marker_path, legacy(&[("machine-a", &gone)])).expect("marker");
        let notes = check_as(
            &on("machine-a", "host", &config),
            &config,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("a gone owner is replaced");
        assert_eq!(notes.len(), 1, "{notes:?}");
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&marker_path).expect("marker")).expect("json");
        assert_eq!(written["version"], 2, "{written}");
        assert!(written["owners"][0].get("shared").is_none(), "{written}");
        assert_eq!(marker(&base).owners[0].project, config.base_path);
        let validator =
            jsonschema::validator_for(&generated_owner_marker_schema()).expect("schema");
        assert!(validator.is_valid(&written), "{written}");
    }

    /// Превью команды записи на базе другой копии говорит то же предупреждение, что прогон, и
    /// метку не трогает.
    #[test]
    fn a_preview_on_a_base_of_another_copy_warns_like_the_run() {
        let dir = tempdir().expect("tempdir");
        let base = base(dir.path());
        let first = project(&dir.path().join("first"), &base);
        let second = project(&dir.path().join("second"), &base);
        check_as(
            &on("machine-a", "host", &first),
            &first,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("the first copy takes the base");
        let recorded = marker(&base);

        let preview = check_as(
            &on("machine-a", "host", &second),
            &second,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Preview,
        )
        .expect("preview");
        let run = check_as(
            &on("machine-a", "host", &second),
            &second,
            "push",
            BaseAccess::Writes,
            OwnerCheck::Run,
        )
        .expect("run");

        assert_eq!(preview, run);
        assert_eq!(marker(&base), recorded);
    }
}
