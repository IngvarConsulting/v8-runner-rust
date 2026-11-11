//! Файл версий принадлежит раннеру, а в каталоге исходников лежит его расходный экземпляр.
//!
//! `ConfigDumpInfo.xml` описывает отношение одной базы к одному каталогу, но платформа
//! ищет его только в каталоге выгрузки, где его затирает кто угодно: соседняя база,
//! ручная выгрузка, `checkout`. Платформа не проверяет, её ли это опись, и выгрузка по
//! изменившемуся молча считает разницу от чужой точки отсчёта.
//!
//! Поэтому раннер держит копию у себя — `localPath/infobases/<база>/dump-info/<набор>/` —
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

use crate::domain::source_set::SourceSetContext;
use crate::platform::dump_format::{known_format, read_recorded, FormatVersion, RecordedFormat};
use crate::platform::locator::PlatformVersion;
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
#[derive(Debug)]
pub(crate) struct RunnerVersionFile {
    source_set: String,
    in_directory: PathBuf,
    copy: PathBuf,
    identity_file: PathBuf,
    identity: String,
}

/// Что сверка оставит в каталоге набора.
enum Restoration {
    /// Файла в каталоге нет, и он не подкладывается.
    Absent,
    /// Файл каталога остаётся: он совпадает с копией или копии той же пары нет.
    Kept,
    /// Подменённый файл уступает копии раннера.
    FromCopy,
}

impl RunnerVersionFile {
    /// Копия есть у набора с памятью базы. У формата EDT файл версий лежит в снимке
    /// Конфигуратора, который тоже лежит под памятью базы.
    pub(crate) fn of(context: &SourceSetContext) -> Option<Self> {
        let dir = context.version_file_copy_dir()?;
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
    /// раннера. Отпечатка сверка не считает: выгрузке он не нужен, а опись бывает в десятки
    /// мегабайт (#574); кому нужен, спрашивает [`Self::fingerprint_in_directory`].
    pub(crate) fn restore(&self) -> Result<(), AppError> {
        if let Restoration::FromCopy = self.restoration()? {
            copy_atomically(&self.copy, &self.in_directory)?;
            debug!(
                source_set = self.source_set.as_str(),
                path = %self.in_directory.display(),
                "replaced a foreign version file with the runner's copy"
            );
        }
        Ok(())
    }

    /// Отпечаток файла версий в каталоге набора; `None` — файла нет.
    pub(crate) fn fingerprint_in_directory(&self) -> Result<Option<Fingerprint>, AppError> {
        fingerprint(&self.in_directory)
    }

    /// Какой файл версий окажется в каталоге после [`Self::restore`], ничего не меняя:
    /// файл каталога, копия раннера или никакой. Так превью видит то же, что выгрузка.
    pub(crate) fn file_after_restore(&self) -> Result<Option<&Path>, AppError> {
        Ok(match self.restoration()? {
            Restoration::Absent => None,
            Restoration::Kept => Some(&self.in_directory),
            Restoration::FromCopy => Some(&self.copy),
        })
    }

    /// Решение сверки: отсутствующий файл не подкладывается, свой остаётся, подменённый
    /// уступает копии той же пары. Файлы сравниваются байт в байт: отпечатки двух описей в
    /// десятки мегабайт стоили выгрузке по изменившемуся больше секунды (#574).
    fn restoration(&self) -> Result<Restoration, AppError> {
        let Some(present) = open_optional(&self.in_directory)? else {
            return Ok(Restoration::Absent);
        };
        if !self.copy_is_ours()? {
            return Ok(Restoration::Kept);
        }
        let Some(copy) = open_optional(&self.copy)? else {
            return Ok(Restoration::Kept);
        };
        Ok(
            if same_bytes((present, &self.in_directory), (copy, &self.copy))? {
                Restoration::Kept
            } else {
                Restoration::FromCopy
            },
        )
    }

    /// Capture the version file before handing the stage to its publisher.
    /// The private temporary file keeps a large inventory off the heap and
    /// prevents a later user edit from becoming the runner's copy.
    pub(crate) fn prepare_from(&self, stage: &Path) -> Result<Option<File>, AppError> {
        let path = stage.join(VERSION_FILE_NAME);
        let mut input = match File::open(&path) {
            Ok(input) => input,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("read", &path, &error)),
        };
        let mut prepared =
            tempfile::tempfile().map_err(|error| io_error("prepare", &path, &error))?;
        io::copy(&mut input, &mut prepared).map_err(|error| io_error("prepare", &path, &error))?;
        Ok(Some(prepared))
    }

    pub(crate) fn record_prepared(&self, prepared: Option<File>) -> Result<(), AppError> {
        use std::io::{Seek, SeekFrom};
        let Some(mut prepared) = prepared else {
            return Ok(());
        };
        prepared
            .seek(SeekFrom::Start(0))
            .map_err(|error| io_error("read prepared", &self.copy, &error))?;
        self.write_copy_using(
            |path| {
                write_file_atomically(path, |file| io::copy(&mut prepared, file).map(|_| ()))
                    .map_err(|error| io_error("write", path, &error))
            },
            write_identity,
        )
    }

    /// После удачной выгрузки: копией становится то, что записала платформа.
    /// Возвращает предупреждение, если копию записать не удалось.
    #[must_use]
    pub(crate) fn record(&self) -> Option<String> {
        // Every written file becomes the copy, so its fingerprint is not needed: the copy reads
        // the file once instead of hashing it first (#571).
        let recorded = match fs::metadata(&self.in_directory) {
            Ok(_) => self.write_copy(write_identity),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("read", &self.in_directory, &error)),
        };
        recorded.err().map(|error| self.not_recorded(&error))
    }

    /// После удачной загрузки: копия обновляется, только если платформа переписала файл
    /// в каталоге. Загрузка, которая файла версий не пишет, чужой файл своим не делает.
    #[must_use]
    pub(crate) fn record_if_rewritten(&self, before: Option<&Fingerprint>) -> Option<String> {
        self.record_when(|written| before != Some(written))
    }

    /// Копия больше не описывает базу: она стирается, и следующая выгрузка по изменившемуся
    /// не подложит её в каталог. Копии не было — не ошибка.
    pub(crate) fn forget(&self) -> Result<(), AppError> {
        match fs::remove_file(&self.copy) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("remove", &self.copy, &error)),
        }
    }

    fn record_when(&self, rewritten: impl Fn(&Fingerprint) -> bool) -> Option<String> {
        let recorded = fingerprint(&self.in_directory).and_then(|written| match written {
            Some(written) if rewritten(&written) => self.write_copy(write_identity),
            Some(_) | None => Ok(()),
        });
        recorded.err().map(|error| self.not_recorded(&error))
    }

    fn not_recorded(&self, error: &AppError) -> String {
        format!(
            "the runner's copy of {VERSION_FILE_NAME} for source-set '{}' was not updated: {error}; the next pull may dump more than changed",
            self.source_set
        )
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
        self.write_copy_using(
            |path| copy_atomically(&self.in_directory, path),
            identity_writer,
        )
    }

    fn write_copy_using(
        &self,
        copy_writer: impl FnOnce(&Path) -> Result<(), AppError>,
        identity_writer: impl FnOnce(&Path, &str) -> io::Result<()>,
    ) -> Result<(), AppError> {
        let dir = self
            .copy
            .parent()
            .ok_or_else(|| AppError::Runtime(format!("'{}' has no parent", self.copy.display())))?;
        fs::create_dir_all(dir).map_err(|error| io_error("create", dir, &error))?;
        remove_left_candidates(dir)?;
        let ours = self.copy_is_ours()?;
        if !ours {
            match fs::remove_file(&self.identity_file) {
                Ok(()) => {
                    best_effort_fsync_dir(dir).map_err(|error| io_error("sync", dir, &error))?;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(io_error("remove", &self.identity_file, &error)),
            }
        }
        copy_writer(&self.copy)?;
        if !ours {
            identity_writer(&self.identity_file, &self.identity)
                .map_err(|error| io_error("write", &self.identity_file, &error))?;
        }
        Ok(())
    }
}

/// Перед загрузкой: версия формата файла версий не новее той, что пишет платформа.
///
/// Версию, которую пишет платформа, раннер берёт из таблицы замеров
/// ([`known_format`]); для платформы вне таблицы сверки нет и примечания тоже.
pub(crate) fn check_load_format(
    context: &SourceSetContext,
    platform: Option<&PlatformVersion>,
) -> Result<Option<String>, AppError> {
    let Some((platform, written)) = known_format(platform) else {
        return Ok(None);
    };
    check_load_format_against(context, platform, written)
}

/// Сверка с известной версией формата платформы. Версия читается из файла в каталоге
/// набора, а если его там нет — из копии раннера той же пары. Новее — отказ до запуска
/// платформы, называющий обе версии. Файла нет ни там, ни там или версия в нём не
/// распознана — сверки нет, и возвращается примечание о пропуске для ответа.
fn check_load_format_against(
    context: &SourceSetContext,
    platform: &PlatformVersion,
    written: FormatVersion,
) -> Result<Option<String>, AppError> {
    let in_directory = context.path().join(VERSION_FILE_NAME);
    let mut source = in_directory.clone();
    let mut recorded = read_format(&in_directory)?;
    if recorded == RecordedFormat::Missing {
        if let Some(copy) = RunnerVersionFile::of(context) {
            if copy.copy_is_ours()? {
                recorded = read_format(&copy.copy)?;
                source = copy.copy;
            }
        }
    }
    let found = match recorded {
        RecordedFormat::Missing => {
            return Ok(Some(format!(
                "the format version was not checked before the load: no {VERSION_FILE_NAME} in '{}' or in the runner's memory",
                context.path().display()
            )));
        }
        RecordedFormat::Unrecognized => {
            return Ok(Some(format!(
                "the format version was not checked before the load: the format version of '{}' is not recognized",
                source.display()
            )));
        }
        RecordedFormat::Version(found) => found,
    };
    if found > written {
        return Err(AppError::Validation(format!(
            "'{}' is in format {found}, newer than {written} that platform {platform} writes: the platform cannot load it, so the load is refused before it starts; load with a platform that writes {found} or newer",
            source.display()
        )));
    }
    Ok(None)
}

fn read_format(path: &Path) -> Result<RecordedFormat, AppError> {
    read_recorded(path).map_err(|error| io_error("read", path, &error))
}

/// Временные файлы замен файла версий и его копии (`<имя>.candidate-…`), оставленные
/// снятым процессом, в каталоге `dir`. Убираются в начале каждой выгрузки и загрузки
/// набора — под тем же замком, под которым пишутся новые, — и при записи копии: иначе
/// их видит `git status` и сторож замены каталога.
pub(crate) fn remove_left_candidates(dir: &Path) -> Result<(), AppError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("list", dir, &error)),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let left = [VERSION_FILE_NAME, IDENTITY_FILE_NAME]
            .iter()
            .any(|target| {
                name.strip_prefix(target)
                    .is_some_and(|rest| rest.starts_with(ATOMIC_WRITE_CANDIDATE_SUFFIX))
            });
        if left {
            let path = entry.path();
            fs::remove_file(&path).map_err(|error| io_error("remove", &path, &error))?;
        }
    }
    Ok(())
}

fn write_identity(path: &Path, identity: &str) -> io::Result<()> {
    write_file_atomically(path, |file| io::Write::write_all(file, identity.as_bytes()))
}

/// Отпечаток файла без чтения его в память целиком; `None`, если файла нет.
fn fingerprint(path: &Path) -> Result<Option<Fingerprint>, AppError> {
    let Some(mut file) = open_optional(path)? else {
        return Ok(None);
    };
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|error| io_error("read", path, &error))?;
    Ok(Some(hasher.finalize().into()))
}

/// Открытый файл или `None`, если его нет.
fn open_optional(path: &Path) -> Result<Option<File>, AppError> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("open", path, &error)),
    }
}

/// Одно ли содержимое у двух открытых файлов: разная длина отвечает без чтения, иначе файлы
/// читаются вместе до первого расхождения.
fn same_bytes(left: (File, &Path), right: (File, &Path)) -> Result<bool, AppError> {
    use std::io::Read as _;
    let length = |(file, path): &(File, &Path)| {
        file.metadata()
            .map(|meta| meta.len())
            .map_err(|error| io_error("read", path, &error))
    };
    if length(&left)? != length(&right)? {
        return Ok(false);
    }
    let fill = |(file, path): &mut (File, &Path), buffer: &mut [u8]| {
        let mut filled = 0;
        while filled < buffer.len() {
            match file.read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(io_error("read", path, &error)),
            }
        }
        Ok(filled)
    };
    let (mut left, mut right) = (left, right);
    let mut left_block = vec![0_u8; 1 << 16];
    let mut right_block = vec![0_u8; 1 << 16];
    loop {
        let read = fill(&mut left, &mut left_block)?;
        if read != fill(&mut right, &mut right_block)? || left_block[..read] != right_block[..read]
        {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
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
        let context = SourceSetContext::new("main", sources, "designer-main").with_infobase_memory(
            &root.join(".local"),
            "origin",
            identity.to_owned(),
        );
        RunnerVersionFile::of(&context).expect("named base keeps a copy")
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
                .join(".local/infobases/origin/dump-info/main/ConfigDumpInfo.xml")
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
        assert_eq!(
            {
                file.restore().expect("restore");
                file.fingerprint_in_directory().expect("print")
            },
            ours
        );
        assert_eq!(fs::read(&file.in_directory).expect("file"), b"ours");
    }

    /// Сверка сравнивает описи байт в байт, а не отпечатками (#574): подмена той же длины,
    /// отличная только в конце большой описи, тоже уступает копии, а та же опись остаётся.
    #[test]
    fn a_replacement_of_the_same_length_is_told_from_the_copy_byte_by_byte() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let ours: Vec<u8> = (0..200_000_u32).map(|byte| (byte % 251) as u8).collect();
        fs::write(&file.in_directory, &ours).expect("platform wrote");
        assert_eq!(file.record(), None);
        assert_eq!(
            file.file_after_restore().expect("same"),
            Some(file.in_directory.as_path())
        );

        let mut foreign = ours.clone();
        *foreign.last_mut().expect("last byte") ^= 1;
        fs::write(&file.in_directory, &foreign).expect("foreign write");
        assert_eq!(
            file.file_after_restore().expect("replaced"),
            Some(file.copy.as_path())
        );
        file.restore().expect("restore");
        assert_eq!(fs::read(&file.in_directory).expect("file"), ours);
    }

    #[test]
    fn temporary_files_left_by_a_killed_write_are_removed() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let left = root.path().join("sources/ConfigDumpInfo.xml.candidate-x1");
        fs::write(&left, "half").expect("left candidate");
        let kept = root.path().join("sources/Module.bsl");
        fs::write(&kept, "source").expect("source");
        super::remove_left_candidates(&root.path().join("sources")).expect("sweep");
        assert!(!left.exists());
        assert!(kept.exists());

        let copy_dir = file.copy.parent().expect("copy dir").to_path_buf();
        fs::create_dir_all(&copy_dir).expect("copy dir");
        let left_copy = copy_dir.join("ConfigDumpInfo.xml.candidate-x2");
        let left_identity = copy_dir.join("identity.candidate-x3");
        fs::write(&left_copy, "half").expect("left copy");
        fs::write(&left_identity, "half").expect("left identity");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);
        assert!(!left_copy.exists());
        assert!(!left_identity.exists());
    }

    #[test]
    fn a_missing_file_is_not_restored() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);

        fs::remove_file(&file.in_directory).expect("remove");
        assert_eq!(
            {
                file.restore().expect("restore");
                file.fingerprint_in_directory().expect("print")
            },
            None
        );
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
        assert_eq!(
            {
                second.restore().expect("restore");
                second.fingerprint_in_directory().expect("print")
            },
            manual
        );
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

    use crate::platform::dump_format::FormatVersion;

    fn platform_8_3_27() -> crate::platform::locator::PlatformVersion {
        crate::platform::locator::PlatformVersion {
            major: 8,
            minor: 3,
            patch: 27,
            build: 2074,
        }
    }

    fn with_format(version: &str) -> String {
        format!("<ConfigDumpInfo format=\"Hierarchical\" version=\"{version}\"/>")
    }

    /// Механизм сверки на версии формата, переданной тестом: нет файла в каталоге — версия
    /// берётся из копии раннера той же пары; формат новее платформы — отказ с обеими
    /// версиями, не новее — загрузка идёт; нет файла или версия не распознана — пропуск
    /// назван. Платформа из таблицы замеров (#403) сверяется и без версии из теста.
    #[test]
    fn a_load_from_a_newer_format_is_refused_and_the_format_is_read_from_the_runner_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let context = SourceSetContext::new("main", root.path().join("sources"), "designer-main")
            .with_infobase_memory(&root.path().join(".local"), "origin", "base-a".to_owned());
        let platform = platform_8_3_27();
        let check =
            || super::check_load_format_against(&context, &platform, FormatVersion::new(2, 20));

        let skipped = check().expect("no file").expect("the skip is named");
        assert!(skipped.contains("not checked"), "{skipped}");

        fs::write(&file.in_directory, with_format("2.21")).expect("platform wrote");
        assert_eq!(file.record(), None);
        fs::remove_file(&file.in_directory).expect("lost");
        let refusal = check().expect_err("a newer format").to_string();
        assert!(refusal.contains("format 2.21"), "{refusal}");
        assert!(
            refusal.contains("2.20 that platform 8.3.27.2074 writes"),
            "{refusal}"
        );
        let measured = super::check_load_format(&context, Some(&platform))
            .expect_err("8.3.27 writes 2.20 by measurement")
            .to_string();
        assert!(measured.contains("format 2.21"), "{measured}");
        let unmeasured = crate::platform::locator::PlatformVersion {
            minor: 5,
            patch: 2,
            ..platform.clone()
        };
        assert_eq!(
            super::check_load_format(&context, Some(&unmeasured))
                .expect("a platform outside the table is not checked"),
            None
        );

        fs::write(&file.in_directory, with_format("2.17")).expect("older");
        assert_eq!(check().expect("an older format loads"), None);
        fs::write(&file.in_directory, "<ConfigDumpInfo/>").expect("no version");
        let unrecognized = check().expect("unrecognized").expect("the skip is named");
        assert!(unrecognized.contains("not recognized"), "{unrecognized}");
        assert_eq!(
            super::check_load_format(&context, None).expect("unknown platform"),
            None
        );
    }

    /// Превью видит тот файл, который оставит сверка: подменённый уступает копии,
    /// отсутствующий не подкладывается, и ничего при этом не пишется.
    #[test]
    fn the_file_after_restore_is_named_without_writing() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        assert_eq!(file.file_after_restore().expect("absent"), None);
        fs::write(&file.in_directory, "ours").expect("platform wrote");
        assert_eq!(file.record(), None);
        assert_eq!(
            file.file_after_restore().expect("kept"),
            Some(file.in_directory.as_path())
        );
        fs::write(&file.in_directory, "foreign").expect("replaced");
        assert_eq!(
            file.file_after_restore().expect("from copy"),
            Some(file.copy.as_path())
        );
        assert_eq!(fs::read(&file.in_directory).expect("untouched"), b"foreign");
        fs::remove_file(&file.in_directory).expect("lost");
        assert_eq!(file.file_after_restore().expect("not put back"), None);
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
