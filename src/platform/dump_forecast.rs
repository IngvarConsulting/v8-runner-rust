//! Прогноз режима выгрузки по изменившемуся: что платформа сделает с `-update` по этому файлу
//! версий (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
//!
//! Конфигуратор и агент отвечают списком `-getChanges` (`--get-changes`), `ibcmd` — ответом
//! `config export status`. Словари замерены (`references/1c/confirmed-runtime-measurements.md`,
//! разделы о прогнозе режима): от языка интерфейса они не зависят. Решение берётся по ключу
//! строки, а не по прозе; всё, что словарю не принадлежит, — прогноз «неизвестен».

use crate::domain::diff::ChangeKind;

/// Что платформа сделает с выгрузкой по изменившемуся.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpForecast {
    /// Полная выгрузка: `FullDump` / `modified: all`.
    Full,
    /// Выгрузка изменившегося, в том числе пустая.
    Changes,
    /// Ответ вне замеренного словаря или прогноз не получен.
    Unknown,
}

/// Список `-getChanges`: одиночная `FullDump` — полная; только `New:` и `Modified:` или
/// пусто — изменившееся.
pub fn read_changes_list(bytes: &[u8]) -> DumpForecast {
    forecast(parse_changes_list(&String::from_utf8_lossy(bytes)))
}

/// Ответ `ibcmd config export status` в полной форме: `modified: all` — полная; только
/// `added: <объект>` и `modified: <объект>` или пусто — изменившееся.
pub fn read_export_status(text: &str) -> DumpForecast {
    forecast(parse_export_status(text))
}

pub(crate) enum ChangeList<'a> {
    FullDump,
    Objects(Vec<(ChangeKind, &'a str)>),
}

fn forecast(list: Option<ChangeList<'_>>) -> DumpForecast {
    match list {
        Some(ChangeList::FullDump) => DumpForecast::Full,
        Some(ChangeList::Objects(_)) => DumpForecast::Changes,
        None => DumpForecast::Unknown,
    }
}

pub(crate) fn parse_changes_list(text: &str) -> Option<ChangeList<'_>> {
    parse(text, "FullDump", |line| match line.split_once(' ')? {
        ("New:", object) if !object.is_empty() => Some((ChangeKind::Added, object)),
        ("Modified:", object) if !object.is_empty() => Some((ChangeKind::Modified, object)),
        _ => None,
    })
}

pub(crate) fn parse_export_status(text: &str) -> Option<ChangeList<'_>> {
    parse(text, "modified: all", |line| {
        let (key, object) = line.split_once(": ")?;
        if object.is_empty() || object == "all" {
            return None;
        }
        match key {
            "added" => Some((ChangeKind::Added, object)),
            "modified" => Some((ChangeKind::Modified, object)),
            _ => None,
        }
    })
}

fn parse<'a>(
    text: &'a str,
    full_dump: &str,
    parse_line: impl Fn(&'a str) -> Option<(ChangeKind, &'a str)>,
) -> Option<ChangeList<'a>> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .peekable();
    if lines.peek() == Some(&full_dump) {
        lines.next();
        return lines.next().is_none().then_some(ChangeList::FullDump);
    }
    lines
        .map(parse_line)
        .collect::<Option<_>>()
        .map(ChangeList::Objects)
}

#[cfg(test)]
mod tests {
    use super::{read_changes_list, read_export_status, DumpForecast};

    /// Замеренные ответы Конфигуратора и агента (UTF-8 с BOM, CRLF).
    #[test]
    fn the_changes_list_is_read_by_its_measured_keys() {
        assert_eq!(read_changes_list(b"\xEF\xBB\xBF"), DumpForecast::Changes);
        assert_eq!(
            read_changes_list(b"\xEF\xBB\xBFFullDump\r\n"),
            DumpForecast::Full
        );
        assert_eq!(
            read_changes_list(
                "\u{feff}New: Catalog.Справочник9\r\nModified: CommonModule.ОбщийМодуль1\r\n"
                    .as_bytes()
            ),
            DumpForecast::Changes
        );
        assert_eq!(
            read_changes_list(b"Deleted: Catalog.X\r\n"),
            DumpForecast::Unknown
        );
        assert_eq!(
            read_changes_list(b"FullDump\r\nModified: Catalog.X\r\n"),
            DumpForecast::Unknown
        );
        assert_eq!(read_changes_list(b"Modified:\r\n"), DumpForecast::Unknown);
    }

    /// Замеренные ответы `ibcmd` (stdout без BOM и файл `--out` с BOM и CRLF).
    #[test]
    fn the_export_status_is_read_by_its_measured_keys() {
        assert_eq!(read_export_status(""), DumpForecast::Changes);
        assert_eq!(read_export_status("modified: all\n"), DumpForecast::Full);
        assert_eq!(
            read_export_status("\u{feff}modified: all\r\n"),
            DumpForecast::Full
        );
        assert_eq!(
            read_export_status("added: Catalog.Справочник9\nmodified: CommonModule.ОбщийМодуль1\n"),
            DumpForecast::Changes
        );
        assert_eq!(read_export_status("M: all\n"), DumpForecast::Unknown);
        assert_eq!(
            read_export_status("modified: all\nadded: Catalog.X\n"),
            DumpForecast::Unknown
        );
        assert_eq!(
            read_export_status("deleted: Catalog.X\n"),
            DumpForecast::Unknown
        );
    }
}
