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
const LOCAL_LOCK_FILE_NAME: &str = ".lock";

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
        std::fs::create_dir_all(&self.path)?;
        let ignore = self.path.join(LOCAL_DIR_GITIGNORE);
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

    /// Заводит каталог, если `path` лежит в нём: так его готовит писатель, который пишет и вне
    /// `localPath`, — замок и метка файловой базы.
    pub fn prepare_for(&self, path: &Path) -> std::io::Result<()> {
        if path.starts_with(&self.path) {
            self.prepare()?;
        }
        Ok(())
    }
}

/// Файл, который каталог терпит в себе как пустой: `.gitignore` и файлы замков — замка
/// `localPath` и замка файловой базы. Повторный `clone` такой каталог пустым и считает.
pub fn is_preparation_file(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name == LOCAL_DIR_GITIGNORE
            || name.starts_with(LOCAL_LOCK_FILE_NAME)
            || name.contains(".v8-runner.infobase.lock")
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

    #[test]
    fn only_the_gitignore_and_lock_files_count_as_preparation() {
        for name in [
            ".gitignore",
            ".lock",
            ".lock.json",
            ".db.v8-runner.infobase.lock",
            ".db.v8-runner.infobase.lock.json",
        ] {
            assert!(is_preparation_file(std::ffi::OsStr::new(name)), "{name}");
        }
        for name in ["db", "infobases", "notes.txt"] {
            assert!(!is_preparation_file(std::ffi::OsStr::new(name)), "{name}");
        }
    }
}
