//! Запись о службе стенда под `workPath/infobases/<база>/managed/<служба>/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::agent::host_key_fingerprint;
use crate::platform::ibsrv::ServerPorts;
use crate::platform::ragent::ClusterPorts;
use crate::platform::service::{process_facts, ProcessFacts};
use crate::support::fs::write_file_atomically;
use crate::support::machine::{machine_hash, machine_id};

/// Где живёт служба объявленной базы.
#[derive(Debug, Clone)]
pub struct ServicePaths {
    managed: PathBuf,
    root: PathBuf,
    service: String,
}

impl ServicePaths {
    pub fn new(work_path: &Path, infobase: &str, service: &str) -> Self {
        let managed = work_path.join("infobases").join(infobase).join("managed");
        let root = managed.join(service);
        Self {
            managed,
            root,
            service: service.to_owned(),
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

    /// Запись соседней службы той же базы: сервера над этой СУБД.
    pub fn sibling_record(&self, service: &str) -> PathBuf {
        self.managed.join(service).join("record.json")
    }

    /// Каталог `managed/` со своим `.gitignore`: его содержимому в репозитории не место.
    pub fn prepare(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let ignore = self.managed.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(ignore, "*\n")?;
        }
        Ok(())
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
pub fn running_cluster(work_path: &Path, infobase: &str) -> Option<ClusterPins> {
    let paths = ServicePaths::new(work_path, infobase, "cluster");
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
pub fn running_postgres(work_path: &Path, infobase: &str) -> bool {
    let paths = ServicePaths::new(work_path, infobase, "postgres");
    record_names_own_process(&paths.record(), machine_fingerprint().as_deref(), &process_facts)
}

/// Запись называет живой свой процесс. Единственная такая проверка вне `observe_service`: ею
/// отказывают, пока работает другая служба (`one-server-owns-a-database`, `dbms stop`). Запись
/// `starting` без pid процесса не называет.
pub fn record_names_own_process(
    record: &Path,
    machine: Option<&str>,
    facts: &dyn Fn(u32) -> Option<ProcessFacts>,
) -> bool {
    read_json::<ServiceRecord>(record).is_some_and(|record| adopt_recorded_pid(&record, facts, machine))
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
pub fn running_standalone(work_path: &Path, infobase: &str) -> Option<RunningServer> {
    let paths = ServicePaths::new(work_path, infobase, "standalone");
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
    pub fn is_own(&self, facts: Option<&ProcessFacts>, machine: Option<&str>) -> bool {
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
    /// `record_names_own_process`, а не копии `read_json` + `is_own` по сценариям (ревью #538).
    #[test]
    fn only_this_module_tells_an_own_process_by_its_record() {
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        let offenders: Vec<_> = files
            .iter()
            .filter(|path| !path.ends_with("platform/service_record.rs"))
            .filter(|path| std::fs::read_to_string(path).unwrap().contains(".is_own("))
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

    #[test]
    fn the_managed_directory_ignores_its_own_content() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path(), "origin", "postgres");
        paths.prepare().unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.managed().join(".gitignore")).unwrap(),
            "*\n"
        );
    }
}
