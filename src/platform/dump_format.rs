//! Версия формата иерархической выгрузки: что записано в файле версий и что пишет платформа.
//!
//! Версия формата привязана к версии платформы, выгрузка всегда идёт в формате своей
//! платформы, а загрузка принимает формат не новее себя (ИТС, «Руководство
//! разработчика», 2.17.2). Версия записана в корне `ConfigDumpInfo.xml` атрибутом `version`,
//! и её видно до запуска платформы. Сам файл раннер не разбирает: читается только этот
//! атрибут корневого элемента.

use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::Path;

use crate::platform::locator::PlatformVersion;

/// Сколько байт начала файла хватает, чтобы дочитать открывающий тег корня.
const HEAD_LIMIT: u64 = 8 * 1024;

/// Версия формата выгрузки: `<старшая>.<младшая>`, сравнивается по числам.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FormatVersion {
    major: u32,
    minor: u32,
}

impl FormatVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    fn parse(value: &str) -> Option<Self> {
        let (major, minor) = value.trim().split_once('.')?;
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

impl fmt::Display for FormatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Версии формата, которые пишут известные раннеру платформы. Строка попадает сюда только
/// по замеру (`references/1c/confirmed-runtime-measurements.md`, раздел о версии формата
/// файла версий, #403); платформы вне таблицы раннер не угадывает. Конфигуратор, `ibcmd` и
/// агент одной платформы пишут одну и ту же версию, а выгрузка по изменившемуся принимает
/// только её. Строка — выпуск, а не сборка: замерены 8.3.27.2074 и 8.5.4.1306, .1683,
/// .1878, остальные сборки выпуска считаются пишущими ту же версию.
const WRITTEN_FORMATS: &[WrittenFormat] = &[
    WrittenFormat::new((8, 3, 27), FormatVersion::new(2, 20)),
    WrittenFormat::new((8, 5, 4), FormatVersion::new(2, 22)),
];

/// Строка таблицы замеров: выпуск платформы (старшая, младшая версия и выпуск) и версия
/// формата, которую он пишет.
struct WrittenFormat {
    release: (u32, u32, u32),
    format: FormatVersion,
}

impl WrittenFormat {
    const fn new(release: (u32, u32, u32), format: FormatVersion) -> Self {
        Self { release, format }
    }
}

/// Платформа и версия формата, которую она пишет по таблице замеров; `None`, если версия
/// платформы раннеру не видна или её нет в таблице.
pub fn known_format(
    platform: Option<&PlatformVersion>,
) -> Option<(&PlatformVersion, FormatVersion)> {
    platform.and_then(|platform| {
        written_in(WRITTEN_FORMATS, platform).map(|written| (platform, written))
    })
}

fn written_in(table: &[WrittenFormat], platform: &PlatformVersion) -> Option<FormatVersion> {
    table
        .iter()
        .find(|row| row.release == (platform.major, platform.minor, platform.patch))
        .map(|row| row.format)
}

/// Что сказано о версии формата в файле версий.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedFormat {
    /// Файла нет.
    Missing,
    /// Версия прочитана из корня.
    Version(FormatVersion),
    /// Файл есть, но версии формата в его корне раннер не нашёл: такой файл чужой.
    Unrecognized,
}

/// Читает версию формата из корня файла версий, не разбирая остального.
pub fn read_recorded(path: &Path) -> io::Result<RecordedFormat> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RecordedFormat::Missing),
        Err(error) => return Err(error),
    };
    let mut head = Vec::new();
    file.take(HEAD_LIMIT).read_to_end(&mut head)?;
    let head = String::from_utf8_lossy(&head);
    Ok(root_version(&head).map_or(RecordedFormat::Unrecognized, RecordedFormat::Version))
}

/// Атрибут `version` открывающего тега `<ConfigDumpInfo …>`.
fn root_version(head: &str) -> Option<FormatVersion> {
    let start = head.find("<ConfigDumpInfo")?;
    let tag = &head[start..];
    let tag = &tag[..tag.find('>')?];
    let mut rest = tag;
    while let Some(position) = rest.find("version") {
        let preceded = rest[..position]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        let after = rest[position + "version".len()..].trim_start();
        rest = &rest[position + "version".len()..];
        if !preceded {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = &value[1..];
        return FormatVersion::parse(&value[..value.find(quote)?]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        known_format, read_recorded, root_version, written_in, FormatVersion, RecordedFormat,
        WrittenFormat,
    };
    use crate::platform::locator::PlatformVersion;

    #[test]
    fn the_version_is_read_from_the_root_attribute_only() {
        let head = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ConfigDumpInfo xmlns=\"http://v8.1c.ru/8.3/xcf/dumpinfo\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" format=\"Hierarchical\" version=\"2.20\">\n<ConfigVersions>";
        assert_eq!(root_version(head), Some(FormatVersion::new(2, 20)));
        assert_eq!(
            root_version("<ConfigDumpInfo format='Hierarchical' version = '2.9'>"),
            Some(FormatVersion::new(2, 9))
        );
        assert_eq!(root_version("<ConfigDumpInfo xmlns:version=\"x\">"), None);
        assert_eq!(
            root_version("<ConfigDumpInfo format=\"Hierarchical\">"),
            None
        );
        assert_eq!(root_version("<Other version=\"2.20\">"), None);
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(FormatVersion::new(2, 9) < FormatVersion::new(2, 20));
        assert!(FormatVersion::new(3, 0) > FormatVersion::new(2, 20));
        assert_eq!(FormatVersion::new(2, 20).to_string(), "2.20");
    }

    #[test]
    fn a_missing_or_foreign_file_is_named_as_such() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ConfigDumpInfo.xml");
        assert_eq!(read_recorded(&path).expect("read"), RecordedFormat::Missing);
        std::fs::write(&path, "not xml").expect("write");
        assert_eq!(
            read_recorded(&path).expect("read"),
            RecordedFormat::Unrecognized
        );
        std::fs::write(&path, "<ConfigDumpInfo version=\"2.17\">").expect("write");
        assert_eq!(
            read_recorded(&path).expect("read"),
            RecordedFormat::Version(FormatVersion::new(2, 17))
        );
    }

    /// Таблица знает платформу по старшей, младшей версии и выпуску; сборка не важна, а
    /// платформа вне таблицы версии не получает.
    #[test]
    fn only_a_platform_in_the_table_has_a_known_format() {
        let platform = |patch| PlatformVersion {
            major: 8,
            minor: 3,
            patch,
            build: 2074,
        };
        let table = [WrittenFormat::new((8, 3, 27), FormatVersion::new(2, 20))];
        assert_eq!(
            written_in(&table, &platform(27)),
            Some(FormatVersion::new(2, 20))
        );
        assert_eq!(written_in(&table, &platform(26)), None);
    }

    /// Строки таблицы — замеры #403: 8.3.27 пишет 2.20, 8.5.4 — 2.22.
    #[test]
    fn the_table_holds_the_measured_platforms() {
        let platform = |minor, patch, build| PlatformVersion {
            major: 8,
            minor,
            patch,
            build,
        };
        let written = |version: PlatformVersion| known_format(Some(&version)).map(|(_, f)| f);
        assert_eq!(
            written(platform(3, 27, 2074)),
            Some(FormatVersion::new(2, 20))
        );
        assert_eq!(
            written(platform(5, 4, 1878)),
            Some(FormatVersion::new(2, 22))
        );
        assert_eq!(written(platform(5, 1, 1519)), None);
        assert_eq!(known_format(None), None);
    }
}
