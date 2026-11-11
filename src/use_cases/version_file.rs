//! Файл версий принадлежит раннеру и лежит только в его памяти (#646).
//!
//! `ConfigDumpInfo.xml` описывает отношение одной базы к одному каталогу. Раннер держит его
//! в `localPath/infobases/<база>/dump-info/<набор>/` и в каталог набора не кладёт: выгрузка
//! по изменившемуся идёт в пустую стадию по этой копии, а в каталог переносится всё, кроме
//! описи. Файл, оставшийся в каталоге набора от прежних версий раннера, он не читает и не
//! пишет; читает его один раз только переход со старой памяти ([`RunnerVersionFile::usable`]).
//!
//! Копия годится только для той пары, для которой записана: рядом лежат тождество памяти
//! набора и свидетельство дерева — идентификатор каталога набора в файловой системе на
//! момент записи. Каталог, удалённый и созданный заново (новый клон, `rm -rf`), копию не
//! подтверждает, и выгрузка идёт полной. Копия старше дерева безопасна: выгрузка по ней
//! выгрузит больше, чем изменилось, но ничего не пропустит. Поэтому `push` её не трогает, а
//! пишется она только после удачной выгрузки.

use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use tracing::debug;

use crate::domain::source_set::SourceSetContext;
use crate::platform::dump_format::{known_format, read_recorded, FormatVersion, RecordedFormat};
use crate::platform::locator::PlatformVersion;
use crate::support::error::AppError;
use crate::support::fs::{
    best_effort_fsync_dir, read_optional, write_file_atomically, ATOMIC_WRITE_CANDIDATE_SUFFIX,
};

/// Имя описи версий, которую платформа пишет в каталог выгрузки.
pub(crate) const VERSION_FILE_NAME: &str = "ConfigDumpInfo.xml";

/// Тождество пары «база ↔ каталог», для которой записана копия.
const IDENTITY_FILE_NAME: &str = "identity";

/// Свидетельство дерева: идентификатор каталога набора, под которым записана копия.
const TREE_FILE_NAME: &str = "tree";

/// Файл версий одного набора — копия раннера в памяти базы.
#[derive(Debug)]
pub(crate) struct RunnerVersionFile {
    source_set: String,
    tree: PathBuf,
    copy: PathBuf,
    identity_file: PathBuf,
    tree_file: PathBuf,
    identity: String,
}

impl RunnerVersionFile {
    /// Копия есть у набора с памятью базы. У формата EDT каталог набора — снимок
    /// Конфигуратора той же базы.
    pub(crate) fn of(context: &SourceSetContext) -> Option<Self> {
        let dir = context.version_file_copy_dir()?;
        let identity = context.storage_identity()?.to_owned();
        Some(Self {
            source_set: context.name().to_owned(),
            tree: context.path().to_path_buf(),
            copy: dir.join(VERSION_FILE_NAME),
            identity_file: dir.join(IDENTITY_FILE_NAME),
            tree_file: dir.join(TREE_FILE_NAME),
            identity,
        })
    }

    /// Путь копии в памяти базы.
    pub(crate) fn path(&self) -> &Path {
        &self.copy
    }

    /// Копия, по которой выгрузка идёт по изменившемуся: она есть, тождество её пары наше, а
    /// свидетельство дерева совпадает с каталогом набора. `None` — выгрузка будет полной.
    ///
    /// Переход со старой памяти: у копии, записанной прежней версией, свидетельства нет; тогда
    /// раннер один раз сравнивает её с `ConfigDumpInfo.xml` в каталоге набора, который прежняя
    /// сверка держала равным копии. Равны — свидетельство записывается и копия годится; иначе
    /// (файла нет, он другой) — нет, и первая выгрузка полная.
    pub(crate) fn usable(&self) -> Result<Option<&Path>, AppError> {
        self.usable_with(Adoption::Record)
    }

    /// Тот же ответ, что [`Self::usable`], без записи свидетельства: превью и прогноз до
    /// замка следа не оставляют.
    pub(crate) fn would_be_usable(&self) -> Result<Option<&Path>, AppError> {
        self.usable_with(Adoption::Answer)
    }

    fn usable_with(&self, adoption: Adoption) -> Result<Option<&Path>, AppError> {
        if !self.copy_is_ours()? || !self.copy.is_file() {
            return Ok(None);
        }
        let recorded = read_optional(&self.tree_file)
            .map_err(|error| io_error("read", &self.tree_file, &error))?;
        let Some(recorded) = recorded else {
            return self.adopt_left_behind(adoption);
        };
        Ok(
            (tree_witness(&self.tree).as_deref() == Some(recorded.as_slice()))
                .then_some(self.copy.as_path()),
        )
    }

    /// Переход со старой памяти ([`Self::usable`]).
    fn adopt_left_behind(&self, adoption: Adoption) -> Result<Option<&Path>, AppError> {
        let left = self.tree.join(VERSION_FILE_NAME);
        let (Some(left_file), Some(copy_file)) =
            (open_optional(&left)?, open_optional(&self.copy)?)
        else {
            return Ok(None);
        };
        if !same_bytes((left_file, &left), (copy_file, &self.copy))? {
            return Ok(None);
        }
        let Some(witness) = tree_witness(&self.tree) else {
            return Ok(None);
        };
        if adoption == Adoption::Record {
            write_file_atomically(&self.tree_file, |file| io::Write::write_all(file, &witness))
                .map_err(|error| io_error("write", &self.tree_file, &error))?;
            debug!(
                source_set = self.source_set.as_str(),
                "the version file left in the source directory by an earlier runner was adopted"
            );
        }
        Ok(Some(&self.copy))
    }

    /// Захват описи из стадии полной выгрузки до публикации: стадия уходит в каталог без неё,
    /// а копией опись становится только после удачной публикации ([`Self::record_prepared`]).
    /// Частный временный файл держит большую опись вне памяти процесса.
    pub(crate) fn prepare_from(stage: &Path) -> Result<Option<File>, AppError> {
        let path = stage.join(VERSION_FILE_NAME);
        let mut input = match File::open(&path) {
            Ok(input) => input,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("read", &path, &error)),
        };
        let mut prepared =
            tempfile::tempfile().map_err(|error| io_error("prepare", &path, &error))?;
        io::copy(&mut input, &mut prepared).map_err(|error| io_error("prepare", &path, &error))?;
        drop(input);
        fs::remove_file(&path).map_err(|error| io_error("remove", &path, &error))?;
        Ok(Some(prepared))
    }

    /// После удачной публикации: копией становится опись, захваченная [`Self::prepare_from`].
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

    /// После удачной выгрузки в стадию и переноса в каталог: копией становится новая опись
    /// `staged` — переименованием, а с другого тома копированием. Возвращает предупреждение,
    /// если копию записать не удалось.
    #[must_use]
    pub(crate) fn record_from(&self, staged: &Path) -> Option<String> {
        self.write_copy_using(|path| move_atomically(staged, path), write_identity)
            .err()
            .map(|error| self.not_recorded(&error))
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

    /// При смене тождества оно сначала стирается; свидетельство дерева стирается всегда.
    /// Затем пишется опись, свидетельство и только потом новое тождество: оборванная запись
    /// оставляет копию негодной, а не чужую опись своей.
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
            remove_optional(&self.identity_file)?;
        }
        remove_optional(&self.tree_file)?;
        best_effort_fsync_dir(dir).map_err(|error| io_error("sync", dir, &error))?;
        copy_writer(&self.copy)?;
        if let Some(witness) = tree_witness(&self.tree) {
            write_file_atomically(&self.tree_file, |file| io::Write::write_all(file, &witness))
                .map_err(|error| io_error("write", &self.tree_file, &error))?;
        }
        if !ours {
            identity_writer(&self.identity_file, &self.identity)
                .map_err(|error| io_error("write", &self.identity_file, &error))?;
        }
        Ok(())
    }
}

/// Записывает ли переход со старой памяти свидетельство дерева.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adoption {
    Record,
    Answer,
}

/// Свидетельство каталога набора: его идентификатор в файловой системе. `None` — его не
/// узнать; тогда копия не годится.
///
/// Вид записи — `<устройство>:<узел>` на Unix и `<том>:<индекс файла>` на Windows.
fn tree_witness(tree: &Path) -> Option<Vec<u8>> {
    use crate::support::path::FilesystemObjectIdentity;
    let identity = crate::support::path::filesystem_object_identity(tree)
        .inspect_err(|error| {
            debug!(path = %tree.display(), %error, "the identity of the source directory is unknown");
        })
        .ok()?;
    let witness = match identity {
        #[cfg(unix)]
        FilesystemObjectIdentity::Unix { device, inode } => format!("{device}:{inode}"),
        #[cfg(windows)]
        FilesystemObjectIdentity::Windows {
            volume_serial,
            file_index,
        } => format!("{volume_serial}:{file_index}"),
        #[cfg(not(any(unix, windows)))]
        FilesystemObjectIdentity::Portable { .. } => return None,
    };
    Some(witness.into_bytes())
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

/// Сверка с известной версией формата платформы. Версия читается из копии раннера той же
/// пары; файл в каталоге набора не читается. Новее — отказ до запуска платформы, называющий
/// обе версии. Копии нет или версия в ней не распознана — сверки нет, и возвращается
/// примечание о пропуске для ответа.
fn check_load_format_against(
    context: &SourceSetContext,
    platform: &PlatformVersion,
    written: FormatVersion,
) -> Result<Option<String>, AppError> {
    let copy = match RunnerVersionFile::of(context) {
        Some(copy) if copy.copy_is_ours()? => Some(copy.copy),
        Some(_) | None => None,
    };
    let recorded = match &copy {
        Some(copy) => read_format(copy)?,
        None => RecordedFormat::Missing,
    };
    let found = match (recorded, &copy) {
        (RecordedFormat::Version(found), _) => found,
        (RecordedFormat::Unrecognized, Some(copy)) => {
            return Ok(Some(format!(
                "the format version was not checked before the load: the format version of '{}' is not recognized",
                copy.display()
            )));
        }
        (RecordedFormat::Missing | RecordedFormat::Unrecognized, _) => {
            return Ok(Some(format!(
                "the format version was not checked before the load: the runner's memory keeps no {VERSION_FILE_NAME} for source-set '{}'",
                context.name()
            )));
        }
    };
    if found > written {
        let source = copy.as_deref().unwrap_or(context.path());
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

/// Временные файлы замен копии и её спутников (`<имя>.candidate-…`), оставленные снятым
/// процессом, в каталоге копии `dir`. Убираются при каждой записи копии.
fn remove_left_candidates(dir: &Path) -> Result<(), AppError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("list", dir, &error)),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let left = [VERSION_FILE_NAME, IDENTITY_FILE_NAME, TREE_FILE_NAME]
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

fn remove_optional(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove", path, &error)),
    }
}

fn write_identity(path: &Path, identity: &str) -> io::Result<()> {
    write_file_atomically(path, |file| io::Write::write_all(file, identity.as_bytes()))
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

/// Переносит `from` на место `to` одним переименованием; с другого тома — атомарной копией.
fn move_atomically(from: &Path, to: &Path) -> Result<(), AppError> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
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
    use super::RunnerVersionFile;
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

    /// Опись, которую выгрузка оставила в стадии.
    fn staged(root: &Path, content: &str) -> std::path::PathBuf {
        let stage = root.join("stage");
        fs::create_dir_all(&stage).expect("stage");
        let path = stage.join("ConfigDumpInfo.xml");
        fs::write(&path, content).expect("staged description");
        path
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

    /// Новая опись из стадии становится копией и уходит из стадии; в каталог набора раннер её
    /// не кладёт, а файл, оставшийся там, не трогает.
    #[test]
    fn the_staged_description_becomes_the_copy_and_never_reaches_the_sources() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let left = root.path().join("sources/ConfigDumpInfo.xml");
        fs::write(&left, "left by an earlier runner").expect("left file");
        let description = staged(root.path(), "new");
        assert_eq!(file.record_from(&description), None);
        assert_eq!(fs::read(&file.copy).expect("copy"), b"new");
        assert!(!description.exists(), "the description left the stage");
        assert_eq!(
            fs::read(&left).expect("untouched"),
            b"left by an earlier runner"
        );
        assert_eq!(file.usable().expect("usable"), Some(file.copy.as_path()));
    }

    /// Каталог набора, созданный заново (новый клон), копию не подтверждает: выгрузка полная.
    #[test]
    fn a_recreated_source_directory_does_not_confirm_the_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        assert_eq!(file.record_from(&staged(root.path(), "ours")), None);
        assert!(file.usable().expect("usable").is_some());

        let sources = root.path().join("sources");
        // Прежний каталог жив под другим именем: новый не займёт его идентификатор.
        fs::rename(&sources, root.path().join("old-clone")).expect("moved away");
        fs::create_dir_all(&sources).expect("a new clone");
        assert_eq!(file.usable().expect("recreated"), None);
    }

    #[test]
    fn temporary_files_left_by_a_killed_write_are_removed() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let copy_dir = file.copy.parent().expect("copy dir").to_path_buf();
        fs::create_dir_all(&copy_dir).expect("copy dir");
        let left_copy = copy_dir.join("ConfigDumpInfo.xml.candidate-x2");
        let left_identity = copy_dir.join("identity.candidate-x3");
        let left_tree = copy_dir.join("tree.candidate-x4");
        for left in [&left_copy, &left_identity, &left_tree] {
            fs::write(left, "half").expect("left candidate");
        }
        assert_eq!(file.record_from(&staged(root.path(), "ours")), None);
        assert!(!left_copy.exists());
        assert!(!left_identity.exists());
        assert!(!left_tree.exists());
    }

    #[test]
    fn a_copy_of_another_pair_is_not_used() {
        let root = tempfile::tempdir().expect("root");
        let first = version_file(root.path(), "base-a");
        assert_eq!(first.record_from(&staged(root.path(), "of base a")), None);

        let second = version_file(root.path(), "base-b");
        assert_eq!(second.usable().expect("foreign"), None);
        assert!(first.usable().expect("own").is_some());
    }

    /// Запись, оборванная после описи, не делает опись новой пары копией прежней: ни та, ни
    /// другая пара её своей не считает.
    #[test]
    fn an_interrupted_identity_change_leaves_no_pair_owning_the_copy() {
        let root = tempfile::tempdir().expect("root");
        let first = version_file(root.path(), "base-a");
        assert_eq!(first.record_from(&staged(root.path(), "of base a")), None);

        let second = version_file(root.path(), "base-b");
        let description = staged(root.path(), "of base b");
        second
            .write_copy_using(
                |path| super::move_atomically(&description, path),
                |_, _| Err(std::io::Error::other("killed before the identity")),
            )
            .expect_err("the identity write was cut off");
        assert_eq!(fs::read(&second.copy).expect("copy"), b"of base b");

        assert_eq!(first.usable().expect("identity"), None);
        assert_eq!(second.usable().expect("identity"), None);
    }

    /// Переход со старой памяти: копия прежней версии без свидетельства годится, только если
    /// в каталоге набора лежит тот же файл, что прежняя сверка держала равным копии.
    #[test]
    fn a_copy_left_by_an_earlier_runner_is_adopted_only_beside_the_same_file() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let copy_dir = file.copy.parent().expect("copy dir").to_path_buf();
        fs::create_dir_all(&copy_dir).expect("copy dir");
        fs::write(&file.copy, "ours").expect("earlier copy");
        fs::write(copy_dir.join("identity"), "base-a").expect("earlier identity");
        let left = root.path().join("sources/ConfigDumpInfo.xml");

        assert_eq!(file.usable().expect("no file"), None, "nothing to compare");
        fs::write(&left, "foreign").expect("another file");
        assert_eq!(file.usable().expect("another"), None);
        assert!(!copy_dir.join("tree").exists());

        fs::write(&left, "ours").expect("the same file");
        assert_eq!(file.usable().expect("adopted"), Some(file.copy.as_path()));
        assert!(copy_dir.join("tree").is_file(), "adopted once");
        fs::write(&left, "changed after the adoption").expect("ignored now");
        assert_eq!(file.usable().expect("witness"), Some(file.copy.as_path()));
        assert_eq!(
            fs::read(&left).expect("left"),
            b"changed after the adoption"
        );
    }

    #[test]
    fn a_full_dump_takes_the_description_out_of_the_stage_before_publication() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let description = staged(root.path(), "full");
        let prepared =
            RunnerVersionFile::prepare_from(description.parent().expect("stage")).expect("taken");
        assert!(!description.exists(), "the tree is published without it");
        file.record_prepared(prepared).expect("recorded");
        assert_eq!(fs::read(&file.copy).expect("copy"), b"full");
        assert!(file.usable().expect("usable").is_some());
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

    /// Механизм сверки на версии формата, переданной тестом: версия читается из копии раннера,
    /// файл в каталоге набора не читается; формат новее платформы — отказ с обеими версиями, не
    /// новее — загрузка идёт; нет копии или версия не распознана — пропуск назван. Платформа из
    /// таблицы замеров (#403) сверяется и без версии из теста.
    #[test]
    fn a_load_from_a_newer_format_is_refused_and_the_format_is_read_from_the_runner_copy() {
        let root = tempfile::tempdir().expect("root");
        let file = version_file(root.path(), "base-a");
        let context = SourceSetContext::new("main", root.path().join("sources"), "designer-main")
            .with_infobase_memory(&root.path().join(".local"), "origin", "base-a".to_owned());
        let platform = platform_8_3_27();
        let check =
            || super::check_load_format_against(&context, &platform, FormatVersion::new(2, 20));

        fs::write(
            root.path().join("sources/ConfigDumpInfo.xml"),
            with_format("2.99"),
        )
        .expect("left in the sources");
        let skipped = check().expect("no copy").expect("the skip is named");
        assert!(skipped.contains("not checked"), "{skipped}");

        assert_eq!(
            file.record_from(&staged(root.path(), &with_format("2.21"))),
            None
        );
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

        assert_eq!(
            file.record_from(&staged(root.path(), &with_format("2.17"))),
            None
        );
        assert_eq!(check().expect("an older format loads"), None);
        assert_eq!(
            file.record_from(&staged(root.path(), "<ConfigDumpInfo/>")),
            None
        );
        let unrecognized = check().expect("unrecognized").expect("the skip is named");
        assert!(unrecognized.contains("not recognized"), "{unrecognized}");
        assert_eq!(
            super::check_load_format(&context, None).expect("unknown platform"),
            None
        );
    }
}
