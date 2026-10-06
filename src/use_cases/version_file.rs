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

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use tracing::debug;

use crate::config::model::{AppConfig, SourceFormat};
use crate::domain::source_set::SourceSetContext;
use crate::support::error::AppError;
use crate::use_cases::ignored_files::VERSION_FILE_NAME;

/// Тождество пары «база ↔ каталог», для которой записана копия.
const IDENTITY_FILE_NAME: &str = "identity";

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
    /// Копия есть у набора формата Конфигуратора с памятью именованной базы. Каталог
    /// выгрузки формата EDT — служебный снимок в `workPath`, его и так никто, кроме
    /// раннера, не пишет.
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
    /// раннера. Возвращает то, что лежит в каталоге после сверки.
    pub(crate) fn restore(&self) -> Result<Option<Vec<u8>>, AppError> {
        let Some(present) = read_optional(&self.in_directory)? else {
            return Ok(None);
        };
        let Some(copy) = self.read_copy()? else {
            return Ok(Some(present));
        };
        if copy == present {
            return Ok(Some(present));
        }
        write_atomically(&self.in_directory, &copy)?;
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
    pub(crate) fn record_if_rewritten(&self, before: Option<&[u8]>) -> Option<String> {
        self.record_when(|written| before != Some(written))
    }

    fn record_when(&self, rewritten: impl Fn(&[u8]) -> bool) -> Option<String> {
        let recorded = read_optional(&self.in_directory).and_then(|written| match written {
            Some(written) if rewritten(&written) => self.write_copy(&written),
            Some(_) | None => Ok(()),
        });
        recorded.err().map(|error| {
            format!(
                "the runner's copy of {VERSION_FILE_NAME} for source-set '{}' was not updated: {error}; the next pull may dump more than changed",
                self.source_set
            )
        })
    }

    fn read_copy(&self) -> Result<Option<Vec<u8>>, AppError> {
        match read_optional(&self.identity_file)? {
            Some(identity) if identity == self.identity.as_bytes() => read_optional(&self.copy),
            _ => Ok(None),
        }
    }

    /// Сначала опись, затем тождество: оборванная смена тождества оставляет копию чужой,
    /// а не чужую опись своей.
    fn write_copy(&self, bytes: &[u8]) -> Result<(), AppError> {
        if let Some(dir) = self.copy.parent() {
            fs::create_dir_all(dir).map_err(|error| io_error("create", dir, &error))?;
        }
        write_atomically(&self.copy, bytes)?;
        write_atomically(&self.identity_file, self.identity.as_bytes())
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read", path, &error)),
    }
}

/// Запись через временный файл рядом: читатель видит прежний файл или новый целиком.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let dir = path
        .parent()
        .ok_or_else(|| AppError::Runtime(format!("'{}' has no parent", path.display())))?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|error| io_error("create a temporary file next to", path, &error))?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| io_error("write", path, &error))?;
    temp.persist(path)
        .map_err(|error| io_error("replace", path, &error.error))?;
    Ok(())
}

fn io_error(action: &str, path: &Path, error: &std::io::Error) -> AppError {
    AppError::Runtime(format!("failed to {action} '{}': {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::RunnerVersionFile;
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

        fs::write(&file.in_directory, "foreign").expect("foreign write");
        assert_eq!(file.restore().expect("restore"), Some(b"ours".to_vec()));
        assert_eq!(fs::read(&file.in_directory).expect("file"), b"ours");
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
        assert_eq!(
            second.restore().expect("restore"),
            Some(b"of base b".to_vec())
        );
        assert_eq!(fs::read(&second.in_directory).expect("file"), b"of base b");
    }

    #[test]
    fn a_load_that_did_not_rewrite_the_file_keeps_the_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);

        fs::write(&file.in_directory, "foreign").expect("foreign write");
        let before = fs::read(&file.in_directory).expect("file");
        // Загрузка без записи файла версий: копия не перенимает чужой файл.
        assert_eq!(file.record_if_rewritten(Some(&before)), None);
        assert_eq!(fs::read(&file.copy).expect("copy"), b"ours");

        fs::write(&file.in_directory, "loaded").expect("platform rewrote");
        assert_eq!(file.record_if_rewritten(Some(&before)), None);
        assert_eq!(fs::read(&file.copy).expect("copy"), b"loaded");
    }
}
