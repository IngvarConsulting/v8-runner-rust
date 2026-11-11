//! Память рабочей копии о базе для тестов, которые начинают не с первого знакомства.
//!
//! `push` без памяти о базе отказывает (`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`).
//! Тесту, которому нужна обычная отправка, память пишется так, как её оставляет создание базы
//! раннером: запись журнала поколений на каждый набор. Привязка записи повторяет ту, что
//! строит раннер (`SourceSetsService::designer_context`): разойдётся — тест упадёт отказом
//! `untracked_base`, а не пройдёт молча. Токен — сорок нулей от `ibcmd-rs`: инструмента, который
//! поколением не отвечает, поэтому ни один исполнитель его не сверяет — ни своим токеном, ни
//! токеном инструмента записи (#559): запись не подтверждается и не опровергается.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// База, о которой пишется память.
pub enum Base<'a> {
    /// Каталог файловой базы, абсолютный.
    File(&'a Path),
    /// База в кластере: сервер и имя базы, как в `Srvr=`/`Ref=`.
    Server { host: &'a str, reference: &'a str },
    /// Автономный сервер за шлюзом `host:port`.
    Standalone { host: &'a str, port: u16 },
}

/// Набор исходников: имя, назначение (`CONFIGURATION`, `EXTENSION`) и абсолютный корень.
pub struct Set<'a> {
    pub name: &'a str,
    pub purpose: &'a str,
    pub root: &'a Path,
}

impl<'a> Set<'a> {
    pub fn configuration(name: &'a str, root: &'a Path) -> Self {
        Self {
            name,
            purpose: "CONFIGURATION",
            root,
        }
    }

    pub fn extension(name: &'a str, root: &'a Path) -> Self {
        Self {
            name,
            purpose: "EXTENSION",
            root,
        }
    }
}

/// Ключ памяти базы, названной строкой соединения в `--infobase`: `@` и начало SHA-256
/// её адреса, как у `connection_memory_key`.
pub fn ad_hoc_key(base: &Base<'_>) -> String {
    let digest = Sha256::digest(address(base).as_bytes());
    std::iter::once("@".to_owned())
        .chain(digest[..16].iter().map(|byte| format!("{byte:02x}")))
        .collect()
}

fn address(base: &Base<'_>) -> String {
    match base {
        Base::File(dir) => {
            let canonical = canonical(dir);
            format!("file:{} ({})", path_hash(&canonical), canonical.display())
        }
        Base::Server { host, reference } => format!(
            "server:{}\\{}",
            host.to_lowercase(),
            reference.to_lowercase()
        ),
        Base::Standalone { host, port } => format!("standalone:{host}:{port}"),
    }
}

/// Пишет память о базе `key` (имя базы из местного слоя или [`ad_hoc_key`]) под `localPath`
/// по умолчанию проекта, чей `v8project.yaml` лежит в `project`: `<project>/.local/infobases`.
pub fn remember_base(project: &Path, key: &str, base: Base<'_>, sets: &[Set<'_>]) {
    let address = address(&base);
    let records: serde_json::Map<String, serde_json::Value> = sets
        .iter()
        .map(|set| {
            let identity = identity(&address, set);
            (
                set.name.to_owned(),
                serde_json::json!({
                    "token": "0".repeat(40),
                    "tool": "ibcmd-rs",
                    "after": "build",
                    "recorded_at": "2026-10-06T00:00:00Z",
                    "identity": identity,
                }),
            )
        })
        .collect();
    let dir = project.join(".local").join("infobases").join(key);
    std::fs::create_dir_all(&dir).expect("memory dir");
    std::fs::write(
        dir.join("generation.json"),
        serde_json::to_vec_pretty(&records).expect("records"),
    )
    .expect("generation ledger");
}

/// Тождество пары «база ↔ набор», как его строит раннер.
fn identity(address: &str, set: &Set<'_>) -> String {
    format!(
        "{address}; source={}; purpose={}; set={}",
        path_hash(&canonical(set.root)),
        set.purpose,
        set.name
    )
}

/// Копия описи версий набора в памяти базы `key`, как её записала бы прежняя версия раннера:
/// опись и тождество пары, без свидетельства каталога. Такую копию раннер берёт, только если в
/// каталоге набора лежит тот же `ConfigDumpInfo.xml` (переход со старой памяти, #646).
/// Возвращает путь копии.
pub fn remember_version_file_of_an_earlier_runner(
    project: &Path,
    key: &str,
    base: Base<'_>,
    set: &Set<'_>,
    content: &str,
) -> PathBuf {
    let dir = project
        .join(".local")
        .join("infobases")
        .join(key)
        .join("dump-info")
        .join(set.name);
    std::fs::create_dir_all(&dir).expect("copy dir");
    std::fs::write(dir.join("identity"), identity(&address(&base), set)).expect("identity");
    let copy = dir.join("ConfigDumpInfo.xml");
    std::fs::write(&copy, content).expect("copy");
    copy
}

/// Копия описи версий набора, как её оставляет удачная выгрузка: опись, тождество пары и
/// свидетельство каталога набора (`<устройство>:<узел>`). Возвращает путь копии.
#[cfg(unix)]
pub fn remember_version_file(
    project: &Path,
    key: &str,
    base: Base<'_>,
    set: &Set<'_>,
    content: &str,
) -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let copy = remember_version_file_of_an_earlier_runner(project, key, base, set, content);
    let metadata = std::fs::metadata(set.root).expect("set root");
    std::fs::write(
        copy.with_file_name("tree"),
        format!("{}:{}", metadata.dev(), metadata.ino()),
    )
    .expect("tree witness");
    copy
}

/// Каноничный путь, как его строит раннер: ближайший существующий предок и остаток.
fn canonical(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(existing).is_err() {
        rest.push(existing.file_name().expect("component").to_owned());
        existing = existing.parent().expect("existing ancestor");
    }
    let mut canonical = std::fs::canonicalize(existing).expect("canonical");
    for part in rest.into_iter().rev() {
        canonical.push(part);
    }
    canonical
}

fn path_hash(path: &Path) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Память о базе образца контрактных тестов, как после её создания раннером: база `infobase:`
/// (`origin`) — файловая `<dir>/ib`, набор `main` — `<dir>/project/configuration`,
/// `workPath` — `<dir>/work`, проектный файл — в `<dir>`. Без неё `push` и его превью
/// отказывают `untracked_base`.
pub fn remember_sample(dir: &Path) {
    remember_base(
        dir,
        "origin",
        Base::File(&dir.join("ib")),
        &[Set::configuration(
            "main",
            &dir.join("project").join("configuration"),
        )],
    );
}
