//! Запись о процессе раннера, который переживает команду: служба стенда под
//! `localPath/infobases/<база>/managed/<служба>/` или процесс в другом каталоге-корне, например
//! в каталоге состояния учётной записи ОС. Единственный носитель такой записи: её вид, чтение,
//! атомарная замена и опознание своего процесса.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::agent::host_key_fingerprint;
use crate::platform::ibsrv::ServerPorts;
use crate::platform::ragent::ClusterPorts;
use crate::platform::service::{process_facts, ProcessFacts};
use crate::support::fs::write_file_atomically;
use crate::support::local_dir::LocalDir;
use crate::support::machine::{machine_hash, machine_id};
use crate::support::path::{nearest_existing_canonical_path, snapshot_path_identity};

/// Где живёт служба: каталог `<корень>/<служба>/` с записью, журналом и данными.
#[derive(Debug, Clone)]
pub struct ServicePaths {
    managed: PathBuf,
    root: PathBuf,
    service: String,
    /// `localPath`, которому принадлежит служба стенда; у процесса под другим корнем его нет.
    local: Option<LocalDir>,
}

/// Где записана служба стенда: отпечатки канонического пути её каталога и каталога проекта и
/// сам каталог проекта, чтобы его назвать (`INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`).
///
/// Другой каталог службы — запись скопирована вместе с `.local`: она устаревшая, а сервер из неё
/// чужой. Тот же каталог при другом проекте — `localPath` общий у двух проектов: запись живая и
/// чужая, её не трогают ([`ServicePaths::another_project`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceLocation {
    pub root_identity: String,
    pub project_identity: String,
    pub project: String,
}

impl ServicePaths {
    /// Служба объявленной базы под `localPath/infobases/<база>/managed/`.
    pub fn new(local: &LocalDir, infobase: &str, service: &str) -> Self {
        let mut paths = Self::under(
            &local
                .path()
                .join("infobases")
                .join(infobase)
                .join("managed"),
            service,
        );
        paths.local = Some(local.clone());
        paths
    }

    /// Служба под другим корнем: каталог службы — `<managed>/<service>/`, соседние службы
    /// лежат рядом с ним.
    pub fn under(managed: &Path, service: &str) -> Self {
        Self {
            managed: managed.to_path_buf(),
            root: managed.join(service),
            service: service.to_owned(),
            local: None,
        }
    }

    /// Соседняя служба той же базы: сервер над этой СУБД.
    pub fn sibling(&self, service: &str) -> Self {
        let mut sibling = Self::under(&self.managed, service);
        sibling.local = self.local.clone();
        sibling
    }

    /// Место, которое записывает и сверяет служба стенда; `None` — у процесса под другим
    /// корнем.
    pub fn location(&self) -> Option<ServiceLocation> {
        let local = self.local.as_ref()?;
        let identity = |path: &Path| {
            let canonical =
                nearest_existing_canonical_path(path).unwrap_or_else(|_| path.to_path_buf());
            snapshot_path_identity(&canonical)
        };
        Some(ServiceLocation {
            root_identity: identity(&self.root),
            project_identity: identity(local.project()),
            project: local.project().display().to_string(),
        })
    }

    /// Проект, который записал службу в этот же каталог, если это не наш проект: `localPath`
    /// общий. `None` — записи нет, она наша или скопирована из другого каталога.
    pub fn another_project(&self) -> Option<String> {
        let expected = self.location()?;
        let written = read_json::<ServiceRecord>(&self.record())?.location?;
        (written.root_identity == expected.root_identity
            && written.project_identity != expected.project_identity)
            .then_some(written.project)
    }

    /// Запись о службе, если она записана здесь: по копии из чужого `localPath` или проекта
    /// раннер ничего не снимает.
    pub fn own_record(&self) -> Option<ServiceRecord> {
        read_json::<ServiceRecord>(&self.record()).filter(|record| self.wrote(record))
    }

    /// Записана ли запись здесь: у службы стенда место в записи совпало с её местом.
    fn wrote(&self, record: &ServiceRecord) -> bool {
        match self.location() {
            Some(location) => record.location.as_ref().is_some_and(|written| {
                written.root_identity == location.root_identity
                    && written.project_identity == location.project_identity
            }),
            None => true,
        }
    }

    #[cfg(test)]
    pub fn managed(&self) -> &Path {
        &self.managed
    }
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn passfile(&self) -> PathBuf {
        self.root.join("passfile")
    }
    pub fn host_key(&self) -> PathBuf {
        self.root.join("host-key")
    }
    pub fn staged_data(&self) -> PathBuf {
        self.root.join("data.staged")
    }
    pub fn password(&self) -> PathBuf {
        self.root.join("password")
    }
    pub fn log(&self) -> PathBuf {
        self.root.join(format!("{}.log", self.service))
    }
    pub fn record(&self) -> PathBuf {
        self.root.join("record.json")
    }
    pub fn pins(&self) -> PathBuf {
        self.root.join("ports.json")
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Запись соседней службы той же базы: сервера над этой СУБД. Нужна только тестам: команды
    /// находят соседей по записям каталога `managed`.
    #[cfg(test)]
    pub fn sibling_record(&self, service: &str) -> PathBuf {
        self.managed.join(service).join("record.json")
    }

    /// Каталог службы. Служба стенда — первый писатель в `localPath`: каталог заводит
    /// [`LocalDir::prepare`] вместе со своим `.gitignore`.
    pub fn prepare(&self) -> std::io::Result<()> {
        if let Some(local) = &self.local {
            local.prepare()?;
        }
        std::fs::create_dir_all(&self.root)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordState {
    Starting,
    Running,
}

/// Запись о запущенной службе. Читатель видит её целиком: замена атомарна.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceRecord {
    pub state: RecordState,
    pub pid: Option<u32>,
    pub started: Option<u64>,
    pub image: Option<String>,
    pub machine: Option<String>,
    /// Раннер, записавший `starting`: pid и время старта. Он умер — запись больше не верна.
    #[serde(default)]
    pub starter: Option<(u32, u64)>,
    pub port: u16,
    pub explicit: bool,
    /// Каталог утилит: `status` и `stop` работают, даже если `tools.postgres.path` уже не объявлен.
    pub bin: PathBuf,
    pub version: Option<String>,
    /// Живой агент Конфигуратора, которого держит этот процесс-сторож: у служб стенда его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<LiveAgentFacts>,
    /// Где записана служба стенда ([`ServicePaths::location`]). У записи сторожа агента его нет;
    /// у записи службы стенда без него место неизвестно, и она считается устаревшей.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<ServiceLocation>,
}

/// Что запись сторожа говорит о его агенте: по этому команда находит агента и сверяет, тот ли
/// он (`INV.PLATFORM.A-LIVE-AGENT-IS-TOLD-APART-BY-ITS-RECORD`). «Занят» здесь нет: это знает
/// и отвечает по `status` только сторож.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveAgentFacts {
    /// Номер процесса агента; `None` — платформа его не назвала.
    pub pid: Option<u32>,
    /// Время старта процесса агента в единицах системы ([`ProcessFacts::started`]).
    pub started: Option<u64>,
    /// Отпечаток ключа хоста, на котором закреплена сессия сторожа.
    pub fingerprint: Option<String>,
    /// Канал сторожа: имя именованного канала Windows или путь сокета Unix.
    pub channel: String,
    /// Путь базы в каноническом виде: по нему находят живых агентов одной базы.
    pub infobase: String,
    /// `workPath` рабочей копии, которая подняла агента: его называет ожидание `agent hold` на
    /// пределе. У записи прежней версии его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_path: Option<String>,
}

/// Закреплённые порты живут отдельно от записи о запущенной службе: её снятие и замена их не
/// отменяют (`INV.PLATFORM.MANAGED-PORTS-ARE-PINNED`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinnedPorts {
    pub port: u16,
}

/// Закреплённое кластера: порты, каталог реестра и создана ли в нём база. Признак базы живёт с
/// реестром: новый реестр (`cluster start --new-regport`) получает его только после регистрации.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClusterPins {
    pub ports: ClusterPorts,
    /// Имя каталога реестра под `managed/cluster/`.
    pub registry: String,
    #[serde(default)]
    pub registered: bool,
    /// Идёт перенос в новый реестр (`--new-regport`): прежний ещё не удалён. Прерванный перенос
    /// откатывает следующий `cluster start`, а рабочие команды такой кластер не видят.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PreviousRegistry>,
}

/// Реестр, из которого идёт перенос.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviousRegistry {
    pub ports: ClusterPorts,
    pub registry: String,
    pub registered: bool,
}

/// Работающий свой кластер объявленной базы. Посреди переноса реестра его нет.
pub fn running_cluster(local: &LocalDir, infobase: &str) -> Option<ClusterPins> {
    let paths = ServicePaths::new(local, infobase, "cluster");
    let machine = machine_fingerprint();
    let observed = observe_service(&paths, machine.as_deref(), &process_facts, &|record| {
        adopt_recorded_pid(record, &process_facts, machine.as_deref())
    });
    let Observed::Own(_) = observed else {
        return None;
    };
    read_json::<ClusterPins>(&paths.pins()).filter(|pins| pins.previous.is_none())
}

/// Работает ли своя служба PostgreSQL этой базы.
pub fn running_postgres(local: &LocalDir, infobase: &str) -> bool {
    let paths = ServicePaths::new(local, infobase, "postgres");
    record_names_own_process(&paths, machine_fingerprint().as_deref(), &process_facts)
}

/// Запись называет живой свой процесс. Единственная такая проверка вне `observe_service`: ею
/// отказывают, пока работает другая служба (`one-server-owns-a-database`, `dbms stop`). Запись
/// `starting` без pid процесса не называет.
pub fn record_names_own_process(
    paths: &ServicePaths,
    machine: Option<&str>,
    facts: &dyn Fn(u32) -> Option<ProcessFacts>,
) -> bool {
    paths
        .own_record()
        .is_some_and(|record| adopt_recorded_pid(&record, facts, machine))
}

/// Хеш `machine_id`; `None`, если система идентификатора не дала.
pub fn machine_fingerprint() -> Option<String> {
    Some(machine_hash(&machine_id()?))
}

/// Сервер, который поднял раннер этой копии, как его видят остальные команды.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningServer {
    pub ports: ServerPorts,
    pub host_fingerprint: Option<String>,
}

/// Служба, как её видят запись и система.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Nothing,
    /// Запись есть, но процесса из неё нет.
    Stale,
    /// Её прямо сейчас поднимает живой раннер.
    Starting(ServiceRecord),
    Own(ServiceRecord),
}

/// Единственное чтение записи о службе. `adopt` решает судьбу записи `starting`, чей раннер умер:
/// опознанный процесс делает её рабочей.
pub fn observe_service(
    paths: &ServicePaths,
    machine: Option<&str>,
    facts: &dyn Fn(u32) -> Option<ProcessFacts>,
    adopt: &dyn Fn(&mut ServiceRecord) -> bool,
) -> Observed {
    let Some(mut record) = read_json::<ServiceRecord>(&paths.record()) else {
        return Observed::Nothing;
    };
    // Запись из другого места — копия чужого стенда: её процесс не наш, что бы он ни делал.
    if !paths.wrote(&record) {
        return Observed::Stale;
    }
    if record.state == RecordState::Starting {
        let starter_alive = record
            .starter
            .is_some_and(|(pid, started)| facts(pid).is_some_and(|facts| facts.started == started));
        if starter_alive {
            return Observed::Starting(record);
        }
        if record.machine.as_deref() == machine && adopt(&mut record) {
            record.state = RecordState::Running;
            record.starter = None;
            return Observed::Own(record);
        }
        return Observed::Stale;
    }
    let seen = record.pid.and_then(facts);
    if record.is_own(seen.as_ref(), machine) {
        Observed::Own(record)
    } else {
        Observed::Stale
    }
}

/// `adopt` для службы, чей pid записан сразу после запуска: она своя, если всё совпало.
pub fn adopt_recorded_pid(
    record: &ServiceRecord,
    facts: &dyn Fn(u32) -> Option<ProcessFacts>,
    machine: Option<&str>,
) -> bool {
    let seen = record.pid.and_then(facts);
    record.is_own(seen.as_ref(), machine)
}

/// Работающий свой сервер объявленной базы. Адрес такой цели не объявлен, его знает только запись.
pub fn running_standalone(local: &LocalDir, infobase: &str) -> Option<RunningServer> {
    let paths = ServicePaths::new(local, infobase, "standalone");
    let machine = machine_fingerprint();
    let observed = observe_service(&paths, machine.as_deref(), &process_facts, &|record| {
        adopt_recorded_pid(record, &process_facts, machine.as_deref())
    });
    let Observed::Own(_) = observed else {
        return None;
    };
    Some(RunningServer {
        ports: read_json(&paths.pins())?,
        host_fingerprint: host_key_fingerprint(&paths.host_key()),
    })
}

impl ServiceRecord {
    /// Свой ли процесс: совпали pid, время старта, образ и машина. Нет `machine_id` — чужой
    /// (`INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`).
    fn is_own(&self, facts: Option<&ProcessFacts>, machine: Option<&str>) -> bool {
        let (Some(facts), Some(machine)) = (facts, machine) else {
            return false;
        };
        self.machine.as_deref() == Some(machine)
            && self.started == Some(facts.started)
            && self.image.as_deref() == Some(facts.image.as_str())
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Заменяет файл атомарно: `INV.USE-CASES.A-SERVICE-RECORD-IS-REPLACED-ATOMICALLY`.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_file_atomically(path, |file| {
        serde_json::to_writer_pretty(file, value).map_err(std::io::Error::other)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Защита от повторения: «работает ли своя служба» решают `observe_service` и
    /// `record_names_own_process`. `is_own` закрыт в этом модуле, а `adopt_recorded_pid` вне его
    /// живёт только как `adopt` для `observe_service`, рядом с ним (ревью #538).
    fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    fn source_files() -> Vec<PathBuf> {
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        files
    }

    /// Защита от повторения: запись о процессе, переживающем команду, одна. Копия носителя
    /// (`ServicePaths`, `ServiceRecord`, отпечаток машины) под стендом уже жила мёртвым файлом
    /// `managed_stand/record.rs` (#577); живой агент берёт этот носитель с другим корнем
    /// (`ServicePaths::under`), а не заводит свой. Защита сравнивает по именам: копию под другим
    /// именем она не увидит, такую ловит ревью.
    #[test]
    fn the_process_record_has_one_owner() {
        let offenders: Vec<String> = source_files()
            .iter()
            .filter(|path| !path.ends_with("platform/service_record.rs"))
            .filter(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                [
                    "struct ServicePaths",
                    "struct ServiceRecord",
                    "enum RecordState",
                    "fn machine_fingerprint(",
                ]
                .iter()
                .any(|shape| text.contains(shape))
            })
            .map(|path| path.display().to_string())
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }

    #[test]
    fn only_this_module_tells_an_own_process_by_its_record() {
        let files = source_files();
        let offenders: Vec<String> = files
            .iter()
            .filter(|path| !path.ends_with("platform/service_record.rs"))
            .flat_map(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| line.contains("adopt_recorded_pid("))
                    .filter(|(at, _)| {
                        let from = at.saturating_sub(6);
                        !lines[from..*at]
                            .iter()
                            .any(|line| line.contains("observe_service("))
                    })
                    .map(|(at, _)| format!("{}:{}", path.display(), at + 1))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }

    fn record() -> ServiceRecord {
        ServiceRecord {
            state: RecordState::Running,
            pid: Some(1),
            started: Some(10),
            image: Some("postgres.exe".to_owned()),
            machine: Some("m".to_owned()),
            starter: None,
            port: 24432,
            explicit: true,
            bin: PathBuf::from("bin"),
            version: None,
            agent: None,
            location: None,
        }
    }

    fn facts(started: u64, image: &str) -> ProcessFacts {
        ProcessFacts {
            started,
            image: image.to_owned(),
        }
    }

    #[test]
    fn a_process_is_own_only_when_every_part_of_the_identity_matches() {
        let record = record();
        assert!(record.is_own(Some(&facts(10, "postgres.exe")), Some("m")));
        assert!(!record.is_own(Some(&facts(11, "postgres.exe")), Some("m")));
        assert!(!record.is_own(Some(&facts(10, "other.exe")), Some("m")));
        assert!(!record.is_own(Some(&facts(10, "postgres.exe")), Some("other")));
        assert!(!record.is_own(None, Some("m")));
    }

    #[test]
    fn a_machine_without_an_id_makes_every_process_foreign() {
        assert!(!record().is_own(Some(&facts(10, "postgres.exe")), None));
    }

    #[test]
    fn a_record_survives_the_round_trip_and_replaces_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/record.json");
        write_json(&path, &record()).unwrap();
        let mut second = record();
        second.explicit = false;
        write_json(&path, &second).unwrap();
        assert_eq!(read_json::<ServiceRecord>(&path), Some(second));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    fn local(dir: &Path) -> LocalDir {
        LocalDir::new(&dir.join(".local"), dir)
    }

    #[test]
    fn a_service_under_another_root_keeps_the_same_layout() {
        let dir = tempfile::tempdir().unwrap();
        let declared = ServicePaths::new(&local(dir.path()), "origin", "postgres");
        let under = ServicePaths::under(declared.managed(), "postgres");
        assert_eq!(under.record(), declared.record());
        assert_eq!(under.log(), declared.log());
        assert_eq!(
            under.sibling_record("cluster"),
            declared.sibling_record("cluster")
        );

        let state = dir.path().join("agents");
        let agent = ServicePaths::under(&state, "0f1e");
        assert_eq!(agent.record(), state.join("0f1e").join("record.json"));
        assert_eq!(agent.host_key(), state.join("0f1e").join("host-key"));
        assert_eq!(agent.log(), state.join("0f1e").join("0f1e.log"));
    }

    /// Данные стенда лежат под `localPath`, а не под `workPath`
    /// (`INV.PLATFORM.A-MANAGED-SERVER-KEEPS-ITS-OWN-DATA-DIRECTORY`).
    #[test]
    fn a_stand_service_lives_under_the_local_path() {
        let dir = tempfile::tempdir().unwrap();
        let local = local(dir.path());
        for service in ["postgres", "standalone", "cluster", "ras"] {
            let paths = ServicePaths::new(&local, "origin", service);
            let root = dir
                .path()
                .join(".local")
                .join("infobases")
                .join("origin")
                .join("managed")
                .join(service);
            assert_eq!(paths.root(), root.as_path());
            assert_eq!(paths.data(), root.join("data"));
            assert_eq!(paths.record(), root.join("record.json"));
            assert_eq!(paths.pins(), root.join("ports.json"));
        }
    }

    /// Каталог службы заводит `localPath` со своим `.gitignore`; у `managed/` своего нет
    /// (`INV.USE-CASES.THE-LOCAL-DIRECTORY-IGNORES-ITSELF`).
    #[test]
    fn a_stand_service_prepares_the_local_directory_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(&local(dir.path()), "origin", "postgres");
        paths.prepare().unwrap();
        assert!(paths.root().is_dir());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".local").join(".gitignore")).unwrap(),
            "*
"
        );
        assert!(!paths.managed().join(".gitignore").exists());
    }

    /// Запись помнит своё место: в другом `localPath` или проекте она устаревшая, и её процесс,
    /// даже живой и совпавший во всём остальном, не свой.
    #[test]
    fn a_record_copied_to_another_place_names_a_foreign_process() {
        let dir = tempfile::tempdir().unwrap();
        let own = ServicePaths::new(&local(&dir.path().join("a")), "origin", "postgres");
        own.prepare().unwrap();
        let mut written = record();
        written.location = own.location();
        write_json(&own.record(), &written).unwrap();
        let alive = |_| Some(facts(10, "postgres.exe"));
        assert!(matches!(
            observe_service(&own, Some("m"), &alive, &|_| false),
            Observed::Own(_)
        ));
        assert!(record_names_own_process(&own, Some("m"), &alive));

        // Копия `.local` в другой проект.
        let copy = ServicePaths::new(&local(&dir.path().join("b")), "origin", "postgres");
        copy.prepare().unwrap();
        std::fs::copy(own.record(), copy.record()).unwrap();
        assert_eq!(
            observe_service(&copy, Some("m"), &alive, &|_| true),
            Observed::Stale
        );
        assert!(!record_names_own_process(&copy, Some("m"), &alive));
        assert_eq!(copy.own_record(), None);
        assert_eq!(copy.another_project(), None, "a copy is stale, not shared");

        // Тот же `localPath`, другой проект: общий абсолютный каталог.
        let shared = LocalDir::new(&dir.path().join("a").join(".local"), &dir.path().join("c"));
        let elsewhere = ServicePaths::new(&shared, "origin", "postgres");
        assert!(!record_names_own_process(&elsewhere, Some("m"), &alive));
        assert_eq!(
            elsewhere.another_project(),
            Some(dir.path().join("a").display().to_string()),
            "a shared directory names the project that wrote the record"
        );
        assert_eq!(own.another_project(), None);

        // Запись без места у службы стенда — тоже не своя.
        let mut unplaced = written.clone();
        unplaced.location = None;
        write_json(&own.record(), &unplaced).unwrap();
        assert_eq!(
            observe_service(&own, Some("m"), &alive, &|_| true),
            Observed::Stale
        );
    }
}
