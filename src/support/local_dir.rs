//! Каталог личного состояния рабочей копии (`localPath`): данные стендов, файловая база по
//! умолчанию и замок, под которым меняется стенд. Единственный владелец его создания: первый
//! писатель в каталог зовёт [`LocalDir::prepare`], и каталог появляется вместе со своим
//! `.gitignore` (`INV.USE-CASES.THE-LOCAL-DIRECTORY-IGNORES-ITSELF`). Превью сюда не пишет.

use std::path::{Path, PathBuf};

/// Имя файла, которым каталог прячет себя от системы контроля версий.
pub const LOCAL_DIR_GITIGNORE: &str = ".gitignore";
/// Содержимое этого файла: всё в каталоге — не исходник.
const LOCAL_DIR_GITIGNORE_CONTENT: &str = "*\n";
/// Замок команд, меняющих стенд: `<localPath>/.lock`.
pub const LOCAL_LOCK_FILE_NAME: &str = ".lock";

/// `localPath` и проект, которому он принадлежит.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDir {
    path: PathBuf,
    project: PathBuf,
}

impl LocalDir {
    pub fn new(path: &Path, project: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            project: project.to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Каталог проекта: запись службы помнит его, чтобы копия каталога в другом проекте не
    /// приняла чужой сервер за свой.
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Файл замка команд, меняющих стенд.
    pub fn lock_path(&self) -> PathBuf {
        self.path.join(LOCAL_LOCK_FILE_NAME)
    }

    /// Заводит каталог и кладёт в него `.gitignore` со строкой `*`. Файл, который уже лежит
    /// там, — свой или чужой — не перезаписывается.
    pub fn prepare(&self) -> std::io::Result<()> {
        prepare_ignored_dir(&self.path)
    }

    /// Заводит каталог, если `path` лежит в нём: так его готовит писатель, который пишет и вне
    /// `localPath`, — замок и метка файловой базы.
    pub fn prepare_for(&self, path: &Path) -> std::io::Result<()> {
        if path.starts_with(&self.path) {
            self.prepare()?;
        }
        Ok(())
    }
}

/// Заводит каталог раннера и кладёт в него `.gitignore` со строкой `*`; файл, который уже
/// лежит там, — свой или чужой — не перезаписывается. Единственная реализация: ею готовят и
/// `localPath`, и `workPath` (`INV.USE-CASES.THE-LOCAL-DIRECTORY-IGNORES-ITSELF`,
/// `INV.USE-CASES.THE-WORK-PATH-IGNORES-ITSELF`).
pub fn prepare_ignored_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    let ignore = path.join(LOCAL_DIR_GITIGNORE);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ignore)
    {
        Ok(mut file) => {
            use std::io::Write as _;
            file.write_all(LOCAL_DIR_GITIGNORE_CONTENT.as_bytes())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// Имя общего каталога порождённого рабочей копии: в нём `workPath` по умолчанию.
const BUILD_DIR_NAME: &str = ".build";

/// Заводит `workPath` со своим `.gitignore`. Родителя `.build`, которого ещё нет, раннер
/// заводит так же: каталог появился его руками, и прятать его — тоже его дело. Существующий
/// родитель и его `.gitignore` не трогаются.
pub fn prepare_work_dir(work_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = work_path
        .parent()
        .filter(|parent| parent.file_name() == Some(std::ffi::OsStr::new(BUILD_DIR_NAME)))
    {
        if !parent.exists() {
            prepare_ignored_dir(parent)?;
        }
    }
    prepare_ignored_dir(work_path)
}

/// Расширения соседних файлов замка: файл блокировки ОС и запись о держателе.
pub const LOCK_COMPANIONS: [&str; 2] = [".system", ".json"];

/// Файл, который заводит сам каталог: его `.gitignore` и файлы замка `<localPath>/.lock` —
/// сам замок и его соседи.
pub fn is_own_file(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name == LOCAL_DIR_GITIGNORE
            || name == LOCAL_LOCK_FILE_NAME
            || LOCK_COMPANIONS
                .iter()
                .any(|companion| name.strip_suffix(companion) == Some(LOCAL_LOCK_FILE_NAME))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_directory_is_created_with_a_gitignore_of_everything() {
        let dir = tempfile::tempdir().unwrap();
        let local = LocalDir::new(&dir.path().join(".local"), dir.path());
        local.prepare().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".local").join(".gitignore")).unwrap(),
            "*\n"
        );
    }

    #[test]
    fn a_gitignore_already_there_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".local");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(".gitignore"), "mine\n").unwrap();
        LocalDir::new(&path, dir.path()).prepare().unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join(".gitignore")).unwrap(),
            "mine\n"
        );
    }

    #[test]
    fn a_writer_outside_the_local_directory_creates_nothing_there() {
        let dir = tempfile::tempdir().unwrap();
        let local = LocalDir::new(&dir.path().join(".local"), dir.path());
        local
            .prepare_for(&dir.path().join("build").join("ib"))
            .unwrap();
        assert!(!dir.path().join(".local").exists());
        local
            .prepare_for(&dir.path().join(".local").join("db"))
            .unwrap();
        assert!(dir.path().join(".local").join(".gitignore").is_file());
    }

    /// `workPath` по умолчанию появляется вместе со своим `.gitignore` и `.gitignore` родителя
    /// `.build`, которого не было; существующие файлы не переписываются.
    #[test]
    fn the_work_path_and_a_build_parent_it_creates_ignore_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join(".build").join("v8-runner");
        prepare_work_dir(&work).unwrap();
        for ignored in [&work, &dir.path().join(".build")] {
            assert_eq!(
                std::fs::read_to_string(ignored.join(".gitignore")).unwrap(),
                "*
"
            );
        }

        let other = tempfile::tempdir().unwrap();
        let build = other.path().join(".build");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(
            build.join(".gitignore"),
            "unica
",
        )
        .unwrap();
        prepare_work_dir(&build.join("v8-runner")).unwrap();
        assert_eq!(
            std::fs::read_to_string(build.join(".gitignore")).unwrap(),
            "unica
"
        );
        assert!(build.join("v8-runner").join(".gitignore").is_file());

        // Родитель с другим именем, который уже есть, раннер не помечает.
        let custom = tempfile::tempdir().unwrap();
        prepare_work_dir(&custom.path().join("out").join("work")).unwrap();
        assert!(!custom.path().join("out").join(".gitignore").exists());
        assert!(custom
            .path()
            .join("out")
            .join("work")
            .join(".gitignore")
            .is_file());
    }

    /// `.build`, который уже был без `.gitignore`, остаётся как есть: раннер его не заводил.
    #[test]
    fn an_existing_build_parent_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let build = dir.path().join(".build");
        std::fs::create_dir_all(&build).unwrap();
        prepare_work_dir(&build.join("v8-runner")).unwrap();
        assert!(!build.join(".gitignore").exists());
    }

    #[test]
    fn only_the_gitignore_and_the_lock_files_are_its_own() {
        for name in [".gitignore", ".lock", ".lock.json", ".lock.system"] {
            assert!(is_own_file(std::ffi::OsStr::new(name)), "{name}");
        }
        for name in [".lockdown", ".lock.bak", "db", "infobases", "notes.txt"] {
            assert!(!is_own_file(std::ffi::OsStr::new(name)), "{name}");
        }
    }
}
