use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemObjectIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows { volume_serial: u32, file_index: u64 },
    #[cfg(not(any(unix, windows)))]
    Portable {
        canonical_path: PathBuf,
        created: Option<std::time::SystemTime>,
    },
}

pub fn filesystem_object_identity(path: &Path) -> std::io::Result<FilesystemObjectIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)?;
        Ok(FilesystemObjectIdentity::Unix {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        windows_filesystem_object_identity(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let metadata = std::fs::metadata(path)?;
        Ok(FilesystemObjectIdentity::Portable {
            canonical_path: std::fs::canonicalize(path)?,
            created: metadata.created().ok(),
        })
    }
}

/// Identity of the object an open handle refers to — the same value
/// `filesystem_object_identity` gives for the path that names it. A file removed or
/// replaced after it was opened no longer matches its former name.
pub fn open_file_identity(file: &std::fs::File) -> std::io::Result<FilesystemObjectIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(FilesystemObjectIdentity::Unix {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        windows_handle_identity(file.as_raw_handle())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "open file identity is not available on this platform",
        ))
    }
}

/// Native metadata-change time of an open object. Unlike the writable modification
/// time, this also detects a rewrite whose original bytes and mtime were restored.
#[derive(Debug, PartialEq, Eq)]
pub struct FileChangeTime(i128);

pub fn open_file_change_time(file: &std::fs::File) -> std::io::Result<FileChangeTime> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(FileChangeTime(
            i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()),
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        };
        let mut information = FILE_BASIC_INFO::default();
        // SAFETY: the file retains this valid handle, and the buffer and length
        // describe one FILE_BASIC_INFO that the call only initializes.
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                std::ptr::from_mut(&mut information).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if success == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(FileChangeTime(i128::from(information.ChangeTime)))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "native file change time is not available on this platform",
        ))
    }
}

#[cfg(windows)]
fn windows_filesystem_object_identity(path: &Path) -> std::io::Result<FilesystemObjectIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide_path` is a NUL-terminated UTF-16 buffer that outlives the call; the
    // other pointers are documented as optional and passed as null.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let identity = windows_handle_identity(handle);
    // SAFETY: `handle` was opened above, is valid, and is closed exactly once here.
    unsafe {
        CloseHandle(handle);
    }
    identity
}

#[cfg(windows)]
fn windows_handle_identity(
    handle: windows_sys::Win32::Foundation::HANDLE,
) -> std::io::Result<FilesystemObjectIdentity> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the caller keeps `handle` open for the duration of the call, and
    // `information` is a valid out-pointer the call only writes.
    let success = unsafe { GetFileInformationByHandle(handle, &mut information) };
    if success == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FilesystemObjectIdentity::Windows {
        volume_serial: information.dwVolumeSerialNumber,
        file_index: ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    })
}

/// Том, на котором лежит путь: устройство на Unix, серийный номер тома на Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeId(u64);

impl VolumeId {
    #[cfg(test)]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Том пути или, если его ещё нет, ближайшего существующего предка: каталог, который раннер
/// только создаст, ляжет на том своего родителя.
pub fn volume_of(path: &Path) -> std::io::Result<VolumeId> {
    // Канонический путь несёт и несуществующий остаток: том спрашивается у его
    // ближайшего существующего предка.
    let canonical = nearest_existing_canonical_path(path)?;
    let existing = canonical
        .ancestors()
        .find(|ancestor| std::fs::symlink_metadata(ancestor).is_ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no existing ancestor for path '{}'", path.display()),
            )
        })?;
    match filesystem_object_identity(existing)? {
        #[cfg(unix)]
        FilesystemObjectIdentity::Unix { device, .. } => Ok(VolumeId(device)),
        #[cfg(windows)]
        FilesystemObjectIdentity::Windows { volume_serial, .. } => {
            Ok(VolumeId(u64::from(volume_serial)))
        }
        #[cfg(not(any(unix, windows)))]
        FilesystemObjectIdentity::Portable { .. } => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the volume of a path is not known on this platform",
        )),
    }
}

/// Returns true when `value` can be safely used as a single file/path segment.
pub fn is_safe_path_segment(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    let mut components = Path::new(value).components();
    let is_single_normal_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !is_single_normal_component {
        return false;
    }

    !value.chars().any(|ch| {
        ch.is_control()
            || matches!(
                ch,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
            )
    })
}

pub fn nearest_existing_canonical_path(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    let mut existing = absolute.as_path();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("no existing ancestor for path '{}'", path.display()),
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }

    let existing_canonical = std::fs::canonicalize(existing)?;
    if existing == absolute {
        return Ok(existing_canonical);
    }

    let suffix = absolute
        .strip_prefix(existing)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let mut resolved = existing_canonical;
    for component in suffix.components() {
        // A missing directory may be traversed in the planned path, but a file may not.
        match std::fs::metadata(&resolved) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!("cannot traverse file '{}'", resolved.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match component {
            Component::Normal(part) => {
                resolved.push(part);
                // After `..`, a new component can name an existing symlink. Resolve it
                // before processing the next component; never collapse it lexically.
                match std::fs::symlink_metadata(&resolved) {
                    Ok(_) => resolved = std::fs::canonicalize(&resolved)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Component::ParentDir => {
                resolved.pop(); // At a filesystem root, pop leaves the root unchanged.
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "path '{}' contains unsupported component '{}'",
                        path.display(),
                        component.as_os_str().to_string_lossy()
                    ),
                ));
            }
        }
    }

    Ok(resolved)
}

/// Snapshot bindings compare canonical paths exactly. Unlike lock identities,
/// they must not merge distinct names on case-sensitive volumes or lossy UTF-8 names.
/// The caller resolves filesystem aliases before asking for the binding.
pub fn snapshot_path_identity(path: &Path) -> String {
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
    #[cfg(not(any(unix, windows)))]
    hasher.update(path.as_os_str().as_encoded_bytes());
    format!("{:x}", hasher.finalize())
}

/// A comparison key for caseless names with canonically equivalent spellings.
pub(crate) fn canonical_caseless(value: &str) -> String {
    use unicode_casefold::UnicodeCaseFold;
    use unicode_normalization::UnicodeNormalization;
    value.nfd().case_fold().nfd().collect()
}

pub fn stable_path_identity(path: &Path) -> String {
    let mut hasher = Sha256::new();
    let path = path.to_string_lossy();
    #[cfg(any(windows, target_os = "macos"))]
    let path = canonical_caseless(&path);
    hasher.update(path.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn hashed_lock_path(path: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        )
    })?;
    Ok(parent.join(format!(".{prefix}-{}.lock", stable_path_identity(path))))
}

pub fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
}

const VERBATIM_PREFIX: &str = r"\\?\";
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

pub fn strip_windows_verbatim_prefix(value: &str) -> String {
    if let Some(rest) = value.strip_prefix(VERBATIM_UNC_PREFIX) {
        return format!(r"\\{rest}");
    }

    if let Some(rest) = value.strip_prefix(VERBATIM_PREFIX) {
        return rest.to_owned();
    }

    value.to_owned()
}

/// Текст ответа команды без приставки `\\?\` у путей. Единственный проход на выходе: канонический
/// путь хранит приставку, потому что на его тексте стоят замки и хеш-память, а человеку и
/// агенту она не нужна. Вне Windows такая запись — допустимое имя файла, и текст не меняется.
pub fn response_text(text: &str) -> Cow<'_, str> {
    if cfg!(windows) {
        without_verbatim_prefixes(text, r"\")
    } else {
        Cow::Borrowed(text)
    }
}

/// [`response_text`] для уже сериализованного JSON: там каждая обратная косая записана двумя.
/// Текст, а не `Value`: через `Value` у ответа поменялся бы порядок ключей.
pub fn response_json(json: &str) -> Cow<'_, str> {
    if cfg!(windows) {
        without_verbatim_prefixes(json, r"\\")
    } else {
        Cow::Borrowed(json)
    }
}

/// [`response_text`] для ответа, уже разобранного в `Value`.
pub fn response_value(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(text) => Value::String(response_text(&text).into_owned()),
        Value::Array(items) => Value::Array(items.into_iter().map(response_value).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, field)| (response_text(&key).into_owned(), response_value(field)))
                .collect(),
        ),
        other => other,
    }
}

/// Снимает приставку перед диском и у `UNC`. У `\\?\Volume{…}\` обычного написания нет, и он
/// остаётся как есть. Конца пути в тексте не видно, поэтому имена, которые без приставки
/// значат другое (`con.cf`, `out.`), не различаются: это принятый риск показанного пути.
fn without_verbatim_prefixes<'a>(text: &'a str, backslash: &str) -> Cow<'a, str> {
    let prefix = VERBATIM_PREFIX.replace('\\', backslash);
    if !text.contains(&prefix) {
        return Cow::Borrowed(text);
    }
    let unc = VERBATIM_UNC_PREFIX.replace('\\', backslash);
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&prefix) {
        plain.push_str(&rest[..at]);
        let found = &rest[at..];
        if let Some(tail) = found.strip_prefix(&unc) {
            plain.push_str(backslash);
            plain.push_str(backslash);
            rest = tail;
        } else {
            let tail = &found[prefix.len()..];
            if !names_a_drive(tail, backslash) {
                plain.push_str(&prefix);
            }
            rest = tail;
        }
    }
    plain.push_str(rest);
    Cow::Owned(plain)
}

fn names_a_drive(text: &str, backslash: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && text[2..].starts_with(backslash)
}

/// Путь, который процессу без приставки `\\?\` не передать (`\\?\Volume{…}\`). Называет только
/// сам путь, а не аргумент целиком: в том же аргументе бывает пароль строки соединения.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the path '{path}' cannot be passed to a process without the \\\\?\\ prefix")]
pub struct UnexpressiblePath {
    /// Путь от приставки до `"`, `;` или конца аргумента.
    pub path: String,
}

/// Аргументы и рабочий каталог дочернего процесса в том виде, в каком он их получит.
#[derive(Debug)]
pub struct ChildLine<'a> {
    pub args: Vec<Cow<'a, str>>,
    pub workdir: Option<Cow<'a, Path>>,
}

/// Единственный владелец правила для всех запусков раннера
/// (`INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX`): аргументы и рабочий
/// каталог без приставки `\\?\` у путей в них. Конфигуратор и `ibcmd` путь с приставкой не
/// находят («Файл не обнаружен»), а канонический путь раннера её несёт. Путь ищется по всему
/// тексту аргумента, поэтому `File="\\?\E:\ib"` и `--db-path=\\?\E:\ib` тоже очищаются. Путь,
/// который без приставки не выразить, — отказ: такой процесс не запускают. Вне Windows такая
/// запись — допустимое имя файла, и строка не меняется. Рабочий каталог не в UTF-8 остаётся как
/// есть: приставку в нём текстом не распознать.
pub fn child_line<'a>(
    args: &'a [String],
    workdir: Option<&'a Path>,
) -> Result<ChildLine<'a>, UnexpressiblePath> {
    let args = args
        .iter()
        .map(|arg| child_text(arg))
        .collect::<Result<Vec<_>, _>>()?;
    let workdir = match workdir {
        None => None,
        Some(dir) => Some(match dir.to_str() {
            None => Cow::Borrowed(dir),
            Some(text) => match child_text(text)? {
                Cow::Borrowed(_) => Cow::Borrowed(dir),
                Cow::Owned(plain) => Cow::Owned(PathBuf::from(plain)),
            },
        }),
    };
    Ok(ChildLine { args, workdir })
}

fn child_text(text: &str) -> Result<Cow<'_, str>, UnexpressiblePath> {
    if !cfg!(windows) {
        return Ok(Cow::Borrowed(text));
    }
    let plain = without_verbatim_prefixes(text, r"\");
    match plain.find(VERBATIM_PREFIX) {
        None => Ok(plain),
        Some(at) => {
            let rest = &plain[at..];
            let end = rest.find(['"', ';']).unwrap_or(rest.len());
            Err(UnexpressiblePath {
                path: rest[..end].to_owned(),
            })
        }
    }
}

/// Путь не в UTF-8 возвращается как есть: через `display()` он потерял бы байты, а
/// префикс `\\?\` у такого пути не распознать текстом.
pub fn normalize_windows_verbatim_path(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(text) => PathBuf::from(strip_windows_verbatim_prefix(text)),
        None => path.to_path_buf(),
    }
}

/// Единственное правило путей из настроек: абсолютный путь остаётся собой, относительный
/// считается от `base` — каталога основного `v8project.yaml`.
///
/// Приставка `\\?\` снимается с `base` и `path` до сборки: внутри такого пути `/` не
/// разделитель, а `.` — обычное имя, и разбор компонентов оставил бы их как есть. Дальше
/// результат собран из компонентов: внутренние `.` и повторные разделители уходят,
/// разделители становятся родными для ОС. Утилиты платформы получают путь в argv как есть,
/// и `E:\proj\./src/cf` `ibcmd` не прочёл (#4). `..` остаётся: свернуть его лексически
/// значит пройти мимо символьной ссылки.
pub fn resolve_from(base: &Path, path: &Path) -> PathBuf {
    let base = normalize_windows_verbatim_path(base);
    let path = normalize_windows_verbatim_path(path);
    base.join(path).components().collect()
}

/// Абсолютный путь от рабочего каталога процесса тем же правилом, что [`resolve_from`]:
/// абсолютный путь тоже собирается из компонентов, рабочий каталог для него не читается.
/// Для мест, где путь ещё может прийти относительным (настройки, собранные в коде, а не
/// загрузчиком); загрузчик отдаёт пути проекта уже абсолютными.
pub fn absolute_from_current_dir(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(resolve_from(Path::new(""), path));
    }
    Ok(resolve_from(&std::env::current_dir()?, path))
}

/// Абсолютный путь без `.` и `..`, свёрнутый лексически: файловая система не читается, и
/// путь может ещё не существовать. `..` у корня остаётся корнем. Так `std::path::absolute`
/// уже отвечает на Windows; на Unix он `..` сохраняет.
pub fn lexically_normal_absolute(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(normal.components().next_back(), Some(Component::Normal(_))) {
                    normal.pop();
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// Путь в том виде, в каком его сравнивают проверки каталогов раннера: ссылки и `..` разрешены
/// по ближайшему существующему предку (файловая система только читается), префикс `\\?\` снят,
/// а на Windows регистр не различается — `SRC` и `src` там один каталог.
pub fn compared_path(path: &Path) -> PathBuf {
    let normal = lexically_normal_absolute(path);
    let resolved = normalize_windows_verbatim_path(
        &nearest_existing_canonical_path(&normal).unwrap_or(normal),
    );
    if cfg!(windows) {
        PathBuf::from(resolved.to_string_lossy().to_lowercase())
    } else {
        resolved
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::response_value;
    use super::{
        filesystem_object_identity, hashed_lock_path, is_filesystem_root, is_safe_path_segment,
        lexically_normal_absolute, nearest_existing_canonical_path,
        normalize_windows_verbatim_path, resolve_from, response_json, response_text,
        stable_path_identity, strip_windows_verbatim_prefix, without_verbatim_prefixes,
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn lexically_normal_absolute_folds_dots_without_reading_the_file_system() {
        assert_eq!(
            lexically_normal_absolute(std::path::Path::new("/missing/proj/./a/../../main.cf")),
            PathBuf::from("/missing/main.cf")
        );
        assert_eq!(
            lexically_normal_absolute(std::path::Path::new("/../out")),
            PathBuf::from("/out")
        );
    }

    #[test]
    fn safe_path_segment_rejects_control_characters() {
        assert!(!is_safe_path_segment("Sales\nAddon"));
        assert!(!is_safe_path_segment("Sales\tAddon"));
    }

    #[test]
    fn nearest_existing_canonical_path_uses_existing_ancestor() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).expect("root");

        let resolved =
            nearest_existing_canonical_path(&root.join("nested").join("target")).expect("resolved");

        assert_eq!(
            resolved,
            std::fs::canonicalize(&root)
                .expect("canonical root")
                .join("nested")
                .join("target")
        );
    }

    #[test]
    fn planned_path_resolves_missing_parent_components_without_writes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("missing/../work");

        assert_eq!(
            nearest_existing_canonical_path(&path).expect("planned path"),
            fs::canonicalize(dir.path())
                .expect("canonical root")
                .join("work")
        );
        assert_eq!(fs::read_dir(dir.path()).expect("entries").count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn planned_path_resolves_symlinks_reached_after_missing_parent_components() {
        let dir = tempdir().expect("tempdir");
        let real = dir.path().join("real/nested");
        fs::create_dir_all(&real).expect("real directory");
        std::os::unix::fs::symlink(&real, dir.path().join("link")).expect("symlink");
        let canonical_real = fs::canonicalize(&real).expect("canonical real");

        for prefix in ["", "missing/../"] {
            assert_eq!(
                nearest_existing_canonical_path(&dir.path().join(format!("{prefix}link/child")))
                    .expect("symlink child"),
                canonical_real.join("child")
            );
            assert_eq!(
                nearest_existing_canonical_path(&dir.path().join(format!("{prefix}link/../child")))
                    .expect("symlink parent"),
                canonical_real.parent().expect("real parent").join("child")
            );
        }
        assert!(!dir.path().join("missing").exists());
        assert!(!real.join("child").exists());
    }

    #[cfg(unix)]
    #[test]
    fn planned_path_rejects_dangling_symlinks_instead_of_treating_them_as_missing() {
        let dir = tempdir().expect("tempdir");
        std::os::unix::fs::symlink(dir.path().join("absent"), dir.path().join("link"))
            .expect("dangling symlink");

        for suffix in ["link", "link/child", "missing/../link/child"] {
            assert_eq!(
                nearest_existing_canonical_path(&dir.path().join(suffix))
                    .expect_err("dangling link")
                    .kind(),
                std::io::ErrorKind::NotFound
            );
        }
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn planned_path_rejects_regular_file_traversal_even_before_parent_components() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("file"), "content").expect("file");

        for suffix in ["file/child", "file/../child", "missing/../file/../child"] {
            assert!(
                nearest_existing_canonical_path(&dir.path().join(suffix)).is_err(),
                "must reject file traversal: {suffix}"
            );
        }
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn planned_path_parent_components_stop_at_filesystem_root() {
        let dir = tempdir().expect("tempdir");
        let canonical_dir = fs::canonicalize(dir.path()).expect("canonical directory");
        let mut path = dir.path().join("missing");
        for _ in 0..canonical_dir.components().count() + 3 {
            path.push("..");
        }

        assert_eq!(
            nearest_existing_canonical_path(&path).expect("root"),
            canonical_dir.ancestors().last().expect("filesystem root")
        );
        assert_eq!(fs::read_dir(dir.path()).expect("entries").count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn stable_path_identity_is_canonical_for_symlinked_paths() {
        let dir = tempdir().expect("tempdir");
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        fs::create_dir_all(&real).expect("real");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let real = std::fs::canonicalize(&real).expect("canonical real");
        let link = std::fs::canonicalize(&link).expect("canonical link");

        assert_eq!(stable_path_identity(&real), stable_path_identity(&link));
    }

    #[test]
    fn hashed_lock_path_uses_parent_directory() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("main");
        let lock_path = hashed_lock_path(&target, "dump").expect("lock path");

        assert_eq!(lock_path.parent(), Some(dir.path()));
        assert!(lock_path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with(".dump-") && name.ends_with(".lock")));
    }

    #[test]
    fn filesystem_object_identity_changes_when_directory_is_replaced() {
        let dir = tempdir().expect("tempdir");
        let observed = dir.path().join("observed");
        fs::create_dir(&observed).expect("observed");
        let before = filesystem_object_identity(&observed).expect("before");

        fs::rename(&observed, dir.path().join("original")).expect("move original");
        fs::create_dir(&observed).expect("replace observed");
        let after = filesystem_object_identity(&observed).expect("after");

        assert_ne!(before, after);
    }

    #[test]
    fn filesystem_root_detection_matches_non_root_paths() {
        let dir = tempdir().expect("tempdir");
        assert!(!is_filesystem_root(dir.path()));
    }

    #[test]
    fn strips_windows_verbatim_drive_prefix() {
        assert_eq!(
            strip_windows_verbatim_prefix(r"\\?\E:\Git_reps\MDM\src\cf"),
            r"E:\Git_reps\MDM\src\cf"
        );
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix() {
        assert_eq!(
            strip_windows_verbatim_prefix(r"\\?\UNC\server\share\ib"),
            r"\\server\share\ib"
        );
    }

    #[test]
    #[cfg(unix)]
    fn resolve_from_drops_current_dir_components_and_keeps_parent_dir() {
        let base = PathBuf::from("/srv/project");

        assert_eq!(
            resolve_from(&base, std::path::Path::new("./src//cf/./")),
            PathBuf::from("/srv/project/src/cf")
        );
        assert_eq!(
            resolve_from(&base.join("."), std::path::Path::new("../shared")),
            PathBuf::from("/srv/project/../shared")
        );
        assert_eq!(
            resolve_from(&base, std::path::Path::new("/opt/./va.epf")),
            PathBuf::from("/opt/va.epf")
        );
        assert_eq!(
            super::absolute_from_current_dir(std::path::Path::new("/opt/./tools//x"))
                .expect("absolute path needs no current directory"),
            PathBuf::from("/opt/tools/x")
        );
    }

    /// Сравнение текстом, а не `PathBuf`: равенство путей на Windows идёт по компонентам и
    /// не отличило бы `E:\proj/src` от `E:\proj\src`.
    #[test]
    #[cfg(windows)]
    fn resolve_from_builds_native_windows_paths() {
        fn resolved(base: &str, path: &str) -> String {
            resolve_from(std::path::Path::new(base), std::path::Path::new(path))
                .into_os_string()
                .into_string()
                .expect("utf-8 path")
        }

        assert_eq!(resolved(r"E:\proj", "./src/cf"), r"E:\proj\src\cf");
        assert_eq!(resolved(r"E:/proj", r"src/cf\x"), r"E:\proj\src\cf\x");
        assert_eq!(resolved(r"\\srv\share\p", r".\x"), r"\\srv\share\p\x");
        assert_eq!(resolved(r"\\?\C:\p", "x"), r"C:\p\x");
        assert_eq!(resolved(r"\\?\C:\proj\./src/cf", ""), r"C:\proj\src\cf");
        assert_eq!(
            resolved(r"E:\proj", r"\\?\C:\proj\./src/cf"),
            r"C:\proj\src\cf"
        );
        assert_eq!(resolved(r"E:\proj", r"..\shared"), r"E:\proj\..\shared");
        assert_eq!(
            resolved(r"E:\proj", r"D:\other\.\va.epf"),
            r"D:\other\va.epf"
        );
    }

    /// Аргумент процесса теряет приставку у пути и там, где путь — часть значения
    /// (`INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX`).
    #[test]
    #[cfg(windows)]
    fn a_process_argument_names_a_path_without_the_verbatim_prefix() {
        let args: Vec<String> = [
            r"\\?\C:\Каталог с пробелами\конфигурация.cf",
            r"\\?\UNC\srv\share\база.dt",
            r#"File="\\?\E:\ib""#,
            r"--db-path=\\?\E:\ib",
            r"/DisableStartupDialogs",
        ]
        .map(str::to_owned)
        .into();
        let line = super::child_line(&args, Some(std::path::Path::new(r"\\?\E:\work")))
            .expect("expressible");
        assert_eq!(
            line.args,
            [
                r"C:\Каталог с пробелами\конфигурация.cf",
                r"\\srv\share\база.dt",
                r#"File="E:\ib""#,
                r"--db-path=E:\ib",
                r"/DisableStartupDialogs",
            ]
        );
        assert_eq!(
            line.workdir.as_deref(),
            Some(std::path::Path::new(r"E:\work"))
        );
    }

    /// Путь тома без буквы без приставки не выразить: процесс такой аргумент не получает, а отказ
    /// называет путь, но не пароль из того же аргумента.
    #[test]
    #[cfg(windows)]
    fn a_volume_path_cannot_reach_a_process() {
        let volume = r"\\?\Volume{0b2c1f4e-1111-2222-3333-444455556666}\ib";
        let args = vec![format!(r#"File="{volume}";Usr="u";Pwd="secret""#)];
        let refused = super::child_line(&args, None).expect_err("a volume path is refused");
        assert_eq!(refused.path, volume);
        assert!(!refused.to_string().contains("secret"), "{refused}");
        assert!(super::child_line(&[], Some(std::path::Path::new(volume))).is_err());
    }

    #[test]
    fn verbatim_prefixes_leave_text_before_a_drive_and_at_a_unc_name() {
        for (text, plain) in [
            (r"\\?\E:\proj\main.cf", r"E:\proj\main.cf"),
            (r"\\?\UNC\srv\share\main.cf", r"\\srv\share\main.cf"),
            (
                r"output '\\?\E:\a b\м.cf' is held by '\\?\UNC\srv\s'",
                r"output 'E:\a b\м.cf' is held by '\\srv\s'",
            ),
            (r"\\?\Volume{1234}\main.cf", r"\\?\Volume{1234}\main.cf"),
            (r"\\?\E:", r"\\?\E:"),
            (r"E:\proj\main.cf", r"E:\proj\main.cf"),
        ] {
            assert_eq!(without_verbatim_prefixes(text, r"\"), plain, "{text}");
            let json = serde_json::to_string(text).expect("json");
            let parsed: String =
                serde_json::from_str(&without_verbatim_prefixes(&json, r"\\")).expect("still JSON");
            assert_eq!(parsed, plain, "{json}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_response_off_windows_keeps_a_file_name_spelled_like_the_prefix() {
        let name = r"/tmp/\\?\E:\file";
        assert_eq!(response_text(name), name);
        let json = serde_json::to_string(name).expect("json");
        assert_eq!(response_json(&json), json);
    }

    #[cfg(windows)]
    #[test]
    fn a_response_on_windows_names_paths_without_the_verbatim_prefix() {
        assert_eq!(response_text(r"at \\?\E:\p"), r"at E:\p");
        assert_eq!(
            response_json(r#"{"output":"\\\\?\\E:\\p"}"#),
            r#"{"output":"E:\\p"}"#
        );
        assert_eq!(
            response_value(serde_json::json!({"output": [r"\\?\E:\p"], "n": 1})),
            serde_json::json!({"output": [r"E:\p"], "n": 1})
        );
    }

    #[test]
    fn normalize_windows_verbatim_path_leaves_regular_paths_unchanged() {
        let path = PathBuf::from("/tmp/project");

        assert_eq!(normalize_windows_verbatim_path(&path), path);
    }
}
