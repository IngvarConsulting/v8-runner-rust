//! Файл версий принадлежит раннеру, а в каталоге исходников лежит его расходный экземпляр.
//!
//! `ConfigDumpInfo.xml` описывает отношение одной базы к одному каталогу, но платформа
//! ищет его только в каталоге выгрузки, где его затирает кто угодно: соседняя база,
//! ручная выгрузка, `checkout`. Платформа не проверяет, её ли это опись, и выгрузка по
//! изменившемуся молча считает разницу от чужой точки отсчёта.
//!
//! Поэтому раннер держит копию у себя — `workPath/infobases/<база>/dump-info/<набор>/` —
//! и перед работой от файла версий сверяет с ней файл в каталоге. Подменённый файл
//! уступает место копии; отсутствующий не подкладывается: о каталоге, из которого опись
//! пропала, копия ничего не доказывает. После удачной команды копией становится то, что
//! записала платформа; при сбое копия остаётся прежней.
//!
//! Копия годится только для той пары, для которой записана: рядом с ней лежит тождество
//! памяти набора, и копию с другим тождеством раннер своей не считает.

use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::debug;

use crate::config::model::{AppConfig, SourceFormat};
use crate::domain::source_set::SourceSetContext;
use crate::support::error::AppError;
use crate::support::fs::{
    best_effort_fsync_dir, read_optional, write_file_atomically, ATOMIC_WRITE_CANDIDATE_SUFFIX,
};
use crate::use_cases::ignored_files::VERSION_FILE_NAME;

/// Тождество пары «база ↔ каталог», для которой записана копия.
const IDENTITY_FILE_NAME: &str = "identity";

/// Отпечаток файла версий: SHA-256 содержимого.
pub(crate) type Fingerprint = [u8; 32];

/// Файл версий одного набора: экземпляр в каталоге исходников и копия раннера.
#[derive(Debug, Clone)]
pub(crate) struct RunnerVersionFile {
    source_set: String,
    in_directory: PathBuf,
    copy: PathBuf,
    identity_file: PathBuf,
    identity: String,
}

impl RunnerVersionFile {
    /// Копия есть у набора формата Конфигуратора с памятью именованной базы. Снимок
    /// формата EDT в `workPath/designer/<набор>` общий для всех баз, и его файл версий
    /// по базам раскладывает #214.
    pub(crate) fn of(config: &AppConfig, context: &SourceSetContext) -> Option<Self> {
        if config.format != SourceFormat::Designer {
            return None;
        }
        Self::for_context(context, &config.work_path)
    }

    fn for_context(context: &SourceSetContext, work_path: &Path) -> Option<Self> {
        let dir = context.version_file_copy_dir(work_path)?;
        let identity = context.storage_identity()?.to_owned();
        Some(Self {
            source_set: context.name().to_owned(),
            in_directory: context.path().join(VERSION_FILE_NAME),
            copy: dir.join(VERSION_FILE_NAME),
            identity_file: dir.join(IDENTITY_FILE_NAME),
            identity,
        })
    }

    /// Перед работой от файла версий: подменённый файл в каталоге заменяется копией
    /// раннера. Возвращает отпечаток того, что лежит в каталоге после сверки.
    pub(crate) fn restore(&self) -> Result<Option<Fingerprint>, AppError> {
        self.remove_left_candidates()?;
        let Some(present) = fingerprint(&self.in_directory)? else {
            return Ok(None);
        };
        if !self.copy_is_ours()? {
            return Ok(Some(present));
        }
        let Some(copy) = fingerprint(&self.copy)? else {
            return Ok(Some(present));
        };
        if copy == present {
            return Ok(Some(present));
        }
        copy_atomically(&self.copy, &self.in_directory)?;
        debug!(
            source_set = self.source_set.as_str(),
            path = %self.in_directory.display(),
            "replaced a foreign version file with the runner's copy"
        );
        Ok(Some(copy))
    }

    /// После удачной выгрузки: копией становится то, что записала платформа.
    /// Возвращает предупреждение, если копию записать не удалось.
    #[must_use]
    pub(crate) fn record(&self) -> Option<String> {
        self.record_when(|_| true)
    }

    /// После удачной загрузки: копия обновляется, только если платформа переписала файл
    /// в каталоге. Загрузка, которая файла версий не пишет, чужой файл своим не делает.
    #[must_use]
    pub(crate) fn record_if_rewritten(&self, before: Option<&Fingerprint>) -> Option<String> {
        self.record_when(|written| before != Some(written))
    }

    fn record_when(&self, rewritten: impl Fn(&Fingerprint) -> bool) -> Option<String> {
        let recorded = fingerprint(&self.in_directory).and_then(|written| match written {
            Some(written) if rewritten(&written) => self.write_copy(write_identity),
            Some(_) | None => Ok(()),
        });
        recorded.err().map(|error| {
            format!(
                "the runner's copy of {VERSION_FILE_NAME} for source-set '{}' was not updated: {error}; the next pull may dump more than changed",
                self.source_set
            )
        })
    }

    fn copy_is_ours(&self) -> Result<bool, AppError> {
        let identity = read_optional(&self.identity_file)
            .map_err(|error| io_error("read", &self.identity_file, &error))?;
        Ok(identity.as_deref() == Some(self.identity.as_bytes()))
    }

    /// При смене тождества оно сначала стирается, затем пишется опись и только потом
    /// новое тождество: оборванная смена оставляет копию ничьей, а не чужую опись своей.
    fn write_copy(
        &self,
        identity_writer: impl FnOnce(&Path, &str) -> io::Result<()>,
    ) -> Result<(), AppError> {
        let dir = self
            .copy
            .parent()
            .ok_or_else(|| AppError::Runtime(format!("'{}' has no parent", self.copy.display())))?;
        fs::create_dir_all(dir).map_err(|error| io_error("create", dir, &error))?;
        let ours = self.copy_is_ours()?;
        if !ours {
            match fs::remove_file(&self.identity_file) {
                Ok(()) => {
                    let _ = best_effort_fsync_dir(dir);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(io_error("remove", &self.identity_file, &error)),
            }
        }
        copy_atomically(&self.in_directory, &self.copy)?;
        if !ours {
            identity_writer(&self.identity_file, &self.identity)
                .map_err(|error| io_error("write", &self.identity_file, &error))?;
        }
        Ok(())
    }

    /// Временные файлы прошлых замен, оставленные снятым процессом, убираются под тем же
    /// замком, под которым пишется новый.
    fn remove_left_candidates(&self) -> Result<(), AppError> {
        let Some(dir) = self.in_directory.parent() else {
            return Ok(());
        };
        let prefix = format!("{VERSION_FILE_NAME}{ATOMIC_WRITE_CANDIDATE_SUFFIX}");
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("list", dir, &error)),
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let path = entry.path();
                fs::remove_file(&path).map_err(|error| io_error("remove", &path, &error))?;
            }
        }
        Ok(())
    }
}

fn write_identity(path: &Path, identity: &str) -> io::Result<()> {
    write_file_atomically(path, |file| io::Write::write_all(file, identity.as_bytes()))
}

/// Отпечаток файла без чтения его в память целиком; `None`, если файла нет.
fn fingerprint(path: &Path) -> Result<Option<Fingerprint>, AppError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("open", path, &error)),
    };
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|error| io_error("read", path, &error))?;
    Ok(Some(hasher.finalize().into()))
}

fn copy_atomically(from: &Path, to: &Path) -> Result<(), AppError> {
    write_file_atomically(to, |file| {
        io::copy(&mut File::open(from)?, file).map(|_| ())
    })
    .map_err(|error| {
        AppError::Runtime(format!(
            "failed to copy '{}' to '{}': {error}",
            from.display(),
            to.display()
        ))
    })
}

fn io_error(action: &str, path: &Path, error: &io::Error) -> AppError {
    AppError::Runtime(format!("failed to {action} '{}': {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, RunnerVersionFile};
    use crate::domain::source_set::SourceSetContext;
    use std::fs;
    use std::path::Path;

    fn version_file(root: &Path, identity: &str) -> RunnerVersionFile {
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        let context = SourceSetContext::new("main", sources, "designer-main")
            .with_infobase_memory("origin", identity.to_owned());
        RunnerVersionFile::for_context(&context, &root.join("work"))
            .expect("named base keeps a copy")
    }

    fn print_of(path: &Path) -> Option<super::Fingerprint> {
        fingerprint(path).expect("fingerprint")
    }

    #[test]
    fn the_copy_lies_under_the_base_and_the_set() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        assert_eq!(
            file.copy,
            root.path()
                .join("work/infobases/origin/dump-info/main/ConfigDumpInfo.xml")
        );
    }

    #[test]
    fn a_replaced_file_gives_way_to_the_runner_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);
        let ours = print_of(&file.in_directory);

        fs::write(&file.in_directory, "foreign").expect("foreign write");
        assert_eq!(file.restore().expect("restore"), ours);
        assert_eq!(fs::read(&file.in_directory).expect("file"), b"ours");
    }

    #[test]
    fn a_restore_removes_temporary_files_left_by_a_killed_write() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let left = root.path().join("sources/ConfigDumpInfo.xml.candidate-x1");
        fs::write(&left, "half").expect("left candidate");
        file.restore().expect("restore");
        assert!(!left.exists());
    }

    #[test]
    fn a_missing_file_is_not_restored() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);

        fs::remove_file(&file.in_directory).expect("remove");
        assert_eq!(file.restore().expect("restore"), None);
        assert!(!file.in_directory.exists());
    }

    #[test]
    fn a_copy_of_another_pair_is_not_restored() {
        let root = tempfile::tempdir().expect("root");
        let first = version_file(root.path(), "base-a");
        fs::write(&first.in_directory, "of base a").expect("platform wrote");
        assert_eq!(first.record(), None);

        let second = version_file(root.path(), "base-b");
        fs::write(&second.in_directory, "of base b").expect("manual dump");
        let manual = print_of(&second.in_directory);
        assert_eq!(second.restore().expect("restore"), manual);
        assert_eq!(fs::read(&second.in_directory).expect("file"), b"of base b");
    }

    /// Смена тождества, оборванная после записи описи, не делает опись новой пары копией
    /// прежней: ни та, ни другая пара её своей не считает.
    #[test]
    fn an_interrupted_identity_change_leaves_no_pair_owning_the_copy() {
        let root = tempfile::tempdir().expect("root");
        let first = version_file(root.path(), "base-a");
        fs::write(&first.in_directory, "of base a").expect("platform wrote");
        assert_eq!(first.record(), None);

        let second = version_file(root.path(), "base-b");
        fs::write(&second.in_directory, "of base b").expect("platform wrote");
        second
            .write_copy(|_, _| Err(std::io::Error::other("killed before the identity")))
            .expect_err("the identity write was cut off");
        assert_eq!(fs::read(&second.copy).expect("copy"), b"of base b");

        fs::write(&first.in_directory, "foreign").expect("foreign write");
        assert!(!first.copy_is_ours().expect("identity"));
        first.restore().expect("restore");
        assert_eq!(fs::read(&first.in_directory).expect("file"), b"foreign");
        assert!(!second.copy_is_ours().expect("identity"));
    }

    #[test]
    fn a_load_that_did_not_rewrite_the_file_keeps_the_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);

        fs::write(&file.in_directory, "foreign").expect("foreign write");
        let before = print_of(&file.in_directory);
        // Загрузка без записи файла версий: копия не перенимает чужой файл.
        assert_eq!(file.record_if_rewritten(before.as_ref()), None);
        assert_eq!(fs::read(&file.copy).expect("copy"), b"ours");

        fs::write(&file.in_directory, "loaded").expect("platform rewrote");
        assert_eq!(file.record_if_rewritten(before.as_ref()), None);
        assert_eq!(fs::read(&file.copy).expect("copy"), b"loaded");
    }
}
