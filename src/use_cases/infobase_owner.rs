//! Метка владельца файловой базы: какая рабочая копия держит базу.
//!
//! Базу, с которой ведут разработку, держит одна рабочая копия
//! (`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`). Кто её держит, записано
//! в метке рядом с каталогом базы, снаружи него: копия каталога базы метку не уносит. Форма
//! метки закреплена `CTR.USE-CASES.INFOBASE-OWNER-MARKER`.
//!
//! Проверку зовёт только граница команды (`use_cases::transport`), сразу после замка базы:
//! команда записи на базе другой живой копии отказывает `InfobaseHeld`, прошедшая проверку
//! команда записи на базе из местного слоя записывает свою копию в метку. Команда чтения
//! метку только читает; превью читает её без замка и ничего не пишет.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::loader::load_declared_infobases;
use crate::config::model::AppConfig;
use crate::domain::next_step::NextStep;
use crate::platform::connection::V8Connection;
use crate::support::fs::publish_file_atomically;
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
    /// Идентификатор машины, который переживает смену имени хоста: `machine-id` у Linux,
    /// аппаратный UUID у macOS, `MachineGuid` у Windows; без него — `host:<имя хоста>`.
    pub machine: String,
    /// Имя хоста на момент записи — для людей; машину называет `machine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Канонический каталог проекта копии: там лежит её местный слой.
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
    machine: String,
    host: Option<String>,
    project: PathBuf,
}

impl ThisCopy {
    /// Копия, которой принадлежит проект `config`, на этой машине.
    fn of(config: &AppConfig) -> Self {
        let host = host_name();
        let machine = machine_id()
            .or_else(|| host.as_ref().map(|host| format!("host:{host}")))
            .unwrap_or_else(|| "unknown".to_owned());
        Self::on(machine, host, &config.base_path)
    }

    fn on(machine: String, host: Option<String>, project: &Path) -> Self {
        Self {
            machine,
            host,
            project: canonical(project),
        }
    }

    fn is(&self, record: &OwnerRecord) -> bool {
        record.machine == self.machine && same_path(&record.project, &self.project)
    }
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
/// другой живой копии, `Runtime` — когда метку не прочитать, не понять или не записать.
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
    let owners = marker.map(|marker| marker.owners).unwrap_or_default();
    let nobody_held_it = owners.is_empty();

    let mut kept = Vec::new();
    let mut alive = Vec::new();
    let mut gone = Vec::new();
    for owner in owners {
        match standing(this, &owner, &base_dir) {
            Standing::This => kept.push(owner),
            Standing::Alive(why) => alive.push((owner, why)),
            Standing::Gone(why) => gone.push((owner, why)),
        }
    }
    if !alive.is_empty() {
        return Err(held_refusal(
            command_name,
            &base_dir,
            &marker_path,
            &alive,
        ));
    }
    // Превью ничего не берёт, а строка соединения подчиняется владельцу, но им не
    // становится — даже на базе без метки или с ушедшим владельцем.
    if check == OwnerCheck::Preview || config.infobase_name.is_none() {
        return Ok(Vec::new());
    }
    if !kept.is_empty() && gone.is_empty() {
        return Ok(Vec::new());
    }

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
            "the infobase '{}' had no owner marker and is now held by this working copy '{}' (owner marker '{}')",
            base_dir.display(),
            this.project.display(),
            marker_path.display()
        ));
    }
    if kept.is_empty() {
        kept.push(OwnerRecord {
            machine: this.machine.clone(),
            host: this.host.clone(),
            project: this.project.clone(),
            shared: false,
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
    /// Местный слой копии не прочитать: она считается живой и несогласной.
    Unreadable(String),
    /// Копия с другой машины: проверить её отсюда нельзя.
    Remote,
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
    if owner.machine != this.machine {
        return Standing::Alive(Alive::Remote);
    }
    if !owner.project.is_dir() {
        return Standing::Gone(Gone::DirectoryIsGone);
    }
    // Объявленный путь разрешается от каталога владельца: у проекта, скопированного
    // целиком, `File=build/ib` указывает на его собственную базу, а не на копию.
    match load_declared_infobases(&owner.project) {
        Err(error) => Standing::Alive(Alive::Unreadable(error.to_string())),
        Ok(declared) => {
            let declares = declared.values().any(|infobase| {
                V8Connection::from_connection_string(&infobase.connection)
                    .file_infobase_dir(&owner.project)
                    .is_some_and(|dir| same_path(&dir, base_dir))
            });
            if declares {
                Standing::Alive(Alive::Declares)
            } else {
                Standing::Gone(Gone::NoLongerDeclares)
            }
        }
    }
}

/// Отказ на базе другой копии: кто её держит, как освободить, где метка и какие выходы есть
/// у этой копии. Следующий шаг — первый и безопасный: своя чистая база.
fn held_refusal(
    command_name: &str,
    base_dir: &Path,
    marker_path: &Path,
    alive: &[(OwnerRecord, Alive)],
) -> UseCaseError {
    let holders = alive
        .iter()
        .map(|(owner, why)| match why {
            Alive::Declares => format!(
                "the working copy '{}' on this machine",
                owner.project.display()
            ),
            Alive::Unreadable(error) => format!(
                "the working copy '{}' on this machine, whose local layer cannot be read ({error}) and which therefore counts as holding it",
                owner.project.display()
            ),
            Alive::Remote => format!(
                "the working copy '{}' on machine '{}'",
                owner.project.display(),
                owner.host.as_deref().unwrap_or(&owner.machine)
            ),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let release = alive
        .iter()
        .map(|(owner, why)| match why {
            Alive::Declares | Alive::Unreadable(_) => format!(
                "remove the infobase from v8project.local.yaml of '{}' or remove that working copy",
                owner.project.display()
            ),
            Alive::Remote => format!(
                "on another machine only by hand: delete its record of '{}' from the owner marker",
                owner.project.display()
            ),
        })
        .collect::<Vec<_>>()
        .join("; ");
    UseCaseError::new(
        UseCaseErrorKind::InfobaseHeld,
        format!(
            "cannot start {command_name}: the infobase '{}' is held by {holders}; a command that writes a development infobase runs only in the working copy that holds it, and repeating it does not help. \
             Ways out for this working copy: its own clean infobase — declare infobases.origin with a connection of its own in v8project.local.yaml and run `v8-runner infobase create`; \
             a copy of the infobase with its data — `infobase create --from <infobase>` (not available yet, #330); \
             a shared infobase — `shared: true` at the infobase in v8project.local.yaml of every working copy (not available yet, #328). \
             To free the infobase: {release}. Owner marker: '{}'",
            base_dir.display(),
            marker_path.display()
        ),
    )
    .with_next(NextStep::command("infobase create"))
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
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(MarkerReadError::Io(error)),
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
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| MarkerReadError::Malformed(error.to_string()))
}

/// Пишет метку заменой файла целиком: читатель без замка видит прежнюю метку или новую.
fn write_marker(path: &Path, marker: &OwnerMarker) -> std::io::Result<()> {
    let mut encoded = serde_json::to_vec_pretty(marker).map_err(std::io::Error::other)?;
    encoded.push(b'\n');
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".tmp.{}", std::process::id()));
    let temp_path = path.with_file_name(temp_name);
    std::fs::write(&temp_path, encoded)?;
    publish_file_atomically(&temp_path, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp_path);
    })
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
        fs::create_dir_all(root).expect("project");
        fs::write(
            root.join("v8project.local.yaml"),
            format!("infobases:\n  origin:\n    connection: 'File={}'\n", base.display()),
        )
        .expect("local layer");
        AppConfig {
            base_path: fs::canonicalize(root).expect("canonical project"),
            work_path: root.join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file(format!("File={}", base.display())),
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
        ThisCopy::on(machine.to_owned(), Some(host.to_owned()), &config.base_path)
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

        check_as(&this, &config, "push", BaseAccess::Writes, OwnerCheck::Preview)
            .expect("preview");
        check_as(&this, &config, "infobase.dump", BaseAccess::Reads, OwnerCheck::Run)
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
}
