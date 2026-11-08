//! Запись о службе стенда под `workPath/infobases/<база>/managed/<служба>/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::service::ProcessFacts;
use crate::support::fs::write_file_atomically;
use crate::support::machine::machine_id;

/// Где живёт служба объявленной базы.
#[derive(Debug, Clone)]
pub(crate) struct ServicePaths {
    managed: PathBuf,
    root: PathBuf,
}

impl ServicePaths {
    pub(crate) fn new(work_path: &Path, infobase: &str, service: &str) -> Self {
        let managed = work_path.join("infobases").join(infobase).join("managed");
        let root = managed.join(service);
        Self { managed, root }
    }

    #[cfg(test)]
    pub(crate) fn managed(&self) -> &Path {
        &self.managed
    }
    pub(crate) fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub(crate) fn staged_data(&self) -> PathBuf {
        self.root.join("data.staged")
    }
    pub(crate) fn password(&self) -> PathBuf {
        self.root.join("password")
    }
    pub(crate) fn log(&self) -> PathBuf {
        self.root.join("postgres.log")
    }
    pub(crate) fn record(&self) -> PathBuf {
        self.root.join("record.json")
    }
    pub(crate) fn pins(&self) -> PathBuf {
        self.root.join("ports.json")
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Запись соседней службы той же базы: сервера над этой СУБД.
    pub(crate) fn sibling_record(&self, service: &str) -> PathBuf {
        self.managed.join(service).join("record.json")
    }

    /// Каталог `managed/` со своим `.gitignore`: его содержимому в репозитории не место.
    pub(crate) fn prepare(&self) -> std::io::Result<()> {
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
pub(crate) enum RecordState {
    Starting,
    Running,
}

/// Запись о запущенной службе. Читатель видит её целиком: замена атомарна.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ServiceRecord {
    pub(crate) state: RecordState,
    pub(crate) pid: Option<u32>,
    pub(crate) started: Option<u64>,
    pub(crate) image: Option<String>,
    pub(crate) machine: Option<String>,
    /// Раннер, записавший `starting`: pid и время старта. Он умер — запись больше не верна.
    #[serde(default)]
    pub(crate) starter: Option<(u32, u64)>,
    pub(crate) port: u16,
    pub(crate) explicit: bool,
    /// Каталог утилит: `status` и `stop` работают, даже если `tools.postgres.path` уже не объявлен.
    pub(crate) bin: PathBuf,
    pub(crate) version: Option<String>,
}

/// Закреплённые порты живут отдельно от записи о запущенной службе: её снятие и замена их не
/// отменяют (`INV.PLATFORM.MANAGED-PORTS-ARE-PINNED`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PinnedPorts {
    pub(crate) port: u16,
}

/// Хеш `machine_id`; `None`, если система идентификатора не дала.
pub(crate) fn machine_fingerprint() -> Option<String> {
    Some(crate::use_cases::infobase_owner::machine_hash(
        &machine_id()?
    ))
}

impl ServiceRecord {
    /// Свой ли процесс: совпали pid, время старта, образ и машина. Нет `machine_id` — чужой
    /// (`INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`).
    pub(crate) fn is_own(&self, facts: Option<&ProcessFacts>, machine: Option<&str>) -> bool {
        let (Some(facts), Some(machine)) = (facts, machine) else {
            return false;
        };
        self.machine.as_deref() == Some(machine)
            && self.started == Some(facts.started)
            && self.image.as_deref() == Some(facts.image.as_str())
    }
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Заменяет файл атомарно: `INV.USE-CASES.A-SERVICE-RECORD-IS-REPLACED-ATOMICALLY`.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
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
