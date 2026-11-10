//! Версия формата иерархической выгрузки: что записано в файле версий и что пишет платформа.
//!
//! Версия формата привязана к версии платформы, выгрузка всегда идёт в формате своей
//! платформы, а загрузка принимает формат не новее себя (ИТС, «Руководство
//! разработчика», 2.17.2). Версия записана в корне `ConfigDumpInfo.xml` атрибутом `version`,
//! и её видно до запуска платформы. Тот же атрибут несёт корень `Configuration.xml` — версию
//! формата самих исходников, которую проверяет загрузка. Файлы раннер не разбирает: читается
//! только этот атрибут корневого элемента. Исключение — опись элементов файла версий
//! ([`read_dump_elements`]): по ней раннер переносит удаления выгрузки агента по SFTP.

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::Path;

use crate::platform::locator::{PlatformVersion, PlatformVersionRequirement};

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
/// только её. Строка — выпуск, а не сборка: замерены 8.3.24.1819, 8.3.25.1633, 8.3.26.1656,
/// 8.3.27.2074, .2342, 8.5.1.1529 и 8.5.4.1306, .1683, .1878, остальные сборки выпуска
/// считаются пишущими ту же версию.
const WRITTEN_FORMATS: &[WrittenFormat] = &[
    WrittenFormat::new((8, 3, 24), FormatVersion::new(2, 17)),
    WrittenFormat::new((8, 3, 25), FormatVersion::new(2, 18)),
    WrittenFormat::new((8, 3, 26), FormatVersion::new(2, 19)),
    WrittenFormat::new((8, 3, 27), FormatVersion::new(2, 20)),
    WrittenFormat::new((8, 5, 1), FormatVersion::new(2, 21)),
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

/// Выпуски из таблицы замеров, которые пишут `format`, — масками выпуска (`8.3.27`),
/// новейшие первыми; пусто, если такого выпуска в таблице нет.
pub fn releases_writing(format: FormatVersion) -> Vec<PlatformVersionRequirement> {
    releases_in(WRITTEN_FORMATS, format)
}

fn releases_in(table: &[WrittenFormat], format: FormatVersion) -> Vec<PlatformVersionRequirement> {
    let mut releases: Vec<(u32, u32, u32)> = table
        .iter()
        .filter(|row| row.format == format)
        .map(|row| row.release)
        .collect();
    releases.sort_unstable_by(|left, right| right.cmp(left));
    releases
        .into_iter()
        .filter_map(|(major, minor, patch)| {
            PlatformVersionRequirement::parse(&format!("{major}.{minor}.{patch}"))
        })
        .collect()
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
    read_root_version(path, "ConfigDumpInfo")
}

/// Полные имена элементов, которые описывает файл версий `ConfigDumpInfo.xml` (атрибут `name`
/// каждого `<Metadata>` на любой глубине): `Catalog.X`, `Catalog.X.Form.F`. Ошибка — файла
/// нет, он не читается (`NotFound` и прочие ошибки чтения) или не разобран (`InvalidData`:
/// корень не `ConfigDumpInfo`, у элемента нет имени, XML испорчен). По такой описи судить, что
/// пропало, нельзя.
pub fn read_dump_elements(path: &Path) -> io::Result<HashSet<String>> {
    use quick_xml::events::Event;

    let invalid = |what: String| io::Error::new(ErrorKind::InvalidData, what);
    let text = std::fs::read_to_string(path)?;
    let mut reader = quick_xml::Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut names = HashSet::new();
    let mut seen_root = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(error.to_string()))?
        {
            Event::Start(tag) | Event::Empty(tag) => {
                if !seen_root {
                    if tag.local_name().as_ref() != b"ConfigDumpInfo" {
                        return Err(invalid("the root is not ConfigDumpInfo".to_owned()));
                    }
                    seen_root = true;
                } else if tag.local_name().as_ref() == b"Metadata" {
                    let name = tag
                        .try_get_attribute("name")
                        .map_err(|error| invalid(error.to_string()))?
                        .ok_or_else(|| invalid("an element without a name".to_owned()))?;
                    let name = name
                        .unescape_value()
                        .map_err(|error| invalid(error.to_string()))?;
                    names.insert(name.into_owned());
                }
            }
            Event::Eof if seen_root => return Ok(names),
            Event::Eof => return Err(invalid("no root element".to_owned())),
            _ => {}
        }
    }
}

/// Читает версию формата исходников из корня `Configuration.xml` (`<MetaDataObject …>`),
/// не разбирая остального.
pub fn read_sources_format(path: &Path) -> io::Result<RecordedFormat> {
    read_root_version(path, "MetaDataObject")
}

fn read_root_version(path: &Path, root: &str) -> io::Result<RecordedFormat> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RecordedFormat::Missing),
        Err(error) => return Err(error),
    };
    let mut head = Vec::new();
    file.take(HEAD_LIMIT).read_to_end(&mut head)?;
    let head = String::from_utf8_lossy(&head);
    Ok(root_version(&head, root).map_or(RecordedFormat::Unrecognized, RecordedFormat::Version))
}

/// Атрибут `version` открывающего тега корня `<root …>`.
fn root_version(head: &str, root: &str) -> Option<FormatVersion> {
    let start = head.find(&format!("<{root}"))?;
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
        known_format, read_dump_elements, read_recorded, read_sources_format, releases_in,
        root_version, written_in, FormatVersion, RecordedFormat, WrittenFormat,
    };
    use crate::platform::locator::PlatformVersion;

    /// Опись элементов читается с вложенных `<Metadata>`; без файла, с чужим корнем и с
    /// испорченным XML описи нет.
    #[test]
    fn the_dump_elements_are_read_at_every_depth() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/designer/configuration/ConfigDumpInfo.xml");
        let names = read_dump_elements(&fixture).expect("the fixture is a version file");
        for name in [
            "Catalog.Справочник1",
            "AccountingRegister.РегистрБухгалтерии1.Resource.Ресурс1",
            "CommonForm.Форма.Form",
        ] {
            assert!(names.contains(name), "{name}");
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ConfigDumpInfo.xml");
        assert_eq!(
            read_dump_elements(&path).map_err(|error| error.kind()),
            Err(std::io::ErrorKind::NotFound),
            "no file"
        );
        std::fs::write(
            &path,
            "\u{feff}<ConfigDumpInfo version=\"2.20\"><ConfigVersions><Metadata name=\"Catalog.A\" id=\"1\"><Metadata name=\"Catalog.A.Form.F\" id=\"2\"/></Metadata></ConfigVersions></ConfigDumpInfo>",
        )
        .expect("write");
        assert_eq!(
            read_dump_elements(&path).expect("read"),
            ["Catalog.A", "Catalog.A.Form.F"].map(str::to_owned).into()
        );
        std::fs::write(&path, "<ConfigDumpInfo version=\"2.20\"/>").expect("write");
        assert!(read_dump_elements(&path).expect("read").is_empty());
        for broken in [
            "<Other><Metadata name=\"Catalog.A\"/></Other>",
            "<ConfigDumpInfo><Metadata name=\"Catalog.A\"></ConfigDumpInfo>",
            "<ConfigDumpInfo><Metadata id=\"1\"/></ConfigDumpInfo>",
        ] {
            std::fs::write(&path, broken).expect("write");
            assert_eq!(
                read_dump_elements(&path).map_err(|error| error.kind()),
                Err(std::io::ErrorKind::InvalidData),
                "{broken}"
            );
        }
    }

    #[test]
    fn the_version_is_read_from_the_root_attribute_only() {
        let head = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ConfigDumpInfo xmlns=\"http://v8.1c.ru/8.3/xcf/dumpinfo\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" format=\"Hierarchical\" version=\"2.20\">\n<ConfigVersions>";
        assert_eq!(
            root_version(head, "ConfigDumpInfo"),
            Some(FormatVersion::new(2, 20))
        );
        assert_eq!(
            root_version(
                "<ConfigDumpInfo format='Hierarchical' version = '2.9'>",
                "ConfigDumpInfo"
            ),
            Some(FormatVersion::new(2, 9))
        );
        assert_eq!(
            root_version("<ConfigDumpInfo xmlns:version=\"x\">", "ConfigDumpInfo"),
            None
        );
        assert_eq!(
            root_version("<ConfigDumpInfo format=\"Hierarchical\">", "ConfigDumpInfo"),
            None
        );
        assert_eq!(
            root_version("<Other version=\"2.20\">", "ConfigDumpInfo"),
            None
        );
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

    /// Строки таблицы — замеры #403 и раздела «Раннер на 8.3.24–8.3.26»: от 8.3.24 (2.17)
    /// до 8.5.4 (2.22); выпуск вне таблицы формата не получает.
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
            written(platform(3, 24, 1819)),
            Some(FormatVersion::new(2, 17))
        );
        assert_eq!(
            written(platform(3, 25, 1633)),
            Some(FormatVersion::new(2, 18))
        );
        assert_eq!(
            written(platform(3, 26, 1656)),
            Some(FormatVersion::new(2, 19))
        );
        assert_eq!(
            written(platform(3, 27, 2074)),
            Some(FormatVersion::new(2, 20))
        );
        assert_eq!(
            written(platform(5, 1, 1529)),
            Some(FormatVersion::new(2, 21))
        );
        assert_eq!(
            written(platform(5, 4, 1878)),
            Some(FormatVersion::new(2, 22))
        );
        assert_eq!(written(platform(5, 2, 1000)), None);
        assert_eq!(known_format(None), None);
    }

    /// Версия формата исходников — атрибут корня `Configuration.xml`; объявление XML
    /// перед ним со своим `version` не в счёт.
    #[test]
    fn the_sources_format_is_read_from_the_configuration_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Configuration.xml");
        assert_eq!(
            read_sources_format(&path).expect("read"),
            RecordedFormat::Missing
        );
        std::fs::write(
            &path,
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\">\n<Configuration uuid=\"x\">",
        )
        .expect("write");
        assert_eq!(
            read_sources_format(&path).expect("read"),
            RecordedFormat::Version(FormatVersion::new(2, 20))
        );
    }

    /// Обратный поиск: формат называет выпуски, которые его пишут, новейшие первыми; формат
    /// вне таблицы выпуска не получает.
    #[test]
    fn a_format_names_the_newest_release_that_writes_it() {
        let table = [
            WrittenFormat::new((8, 3, 26), FormatVersion::new(2, 20)),
            WrittenFormat::new((8, 3, 27), FormatVersion::new(2, 20)),
            WrittenFormat::new((8, 5, 4), FormatVersion::new(2, 22)),
        ];
        let releases: Vec<String> = releases_in(&table, FormatVersion::new(2, 20))
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(releases, ["8.3.27", "8.3.26"]);
        assert!(releases_in(&table, FormatVersion::new(2, 21)).is_empty());
    }
}
