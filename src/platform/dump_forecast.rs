//! Списки изменений платформы: что она выгрузит по этому файлу версий.
//!
//! `ibcmd` отвечает `config export status` — по нему выгрузка `ibcmd` выбирает путь
//! (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`). Конфигуратор отвечает
//! списком `-getChanges` — его читает `diff`; выгрузка Конфигуратора и агента прогноза не
//! спрашивает и режим узнаёт по факту. Словари замерены
//! (`references/1c/confirmed-runtime-measurements.md`, разделы о прогнозе режима): от языка
//! интерфейса они не зависят. Решение берётся по ключу строки, а не по прозе; всё, что словарю
//! не принадлежит, — прогноз «неизвестен».

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

/// A list of `-getChanges` that `diff` reads: a single `FullDump`, or only `New:` and
/// `Modified:` lines. An empty list produced by Designer consists solely of the UTF-8
/// signature; any other empty text is not a list.
pub(crate) fn parse_designer_diff(text: &str) -> Option<ChangeList<'_>> {
    let list = parse(text, "FullDump", |line| match line.split_once(' ')? {
        ("New:", object) if !object.is_empty() => Some((ChangeKind::Added, object)),
        ("Modified:", object) if !object.is_empty() => Some((ChangeKind::Modified, object)),
        _ => None,
    })?;
    match &list {
        ChangeList::Objects(objects)
            if objects.is_empty() && text.as_bytes() != b"\xEF\xBB\xBF" =>
        {
            None
        }
        _ => Some(list),
    }
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
    use super::{parse_designer_diff, read_export_status, ChangeList, DumpForecast};

    /// Замеренные ответы Конфигуратора (UTF-8 с BOM, CRLF), которые читает `diff`: пустой
    /// список — одна сигнатура UTF-8.
    #[test]
    fn only_the_utf8_signature_is_a_valid_empty_designer_diff() {
        assert!(
            matches!(parse_designer_diff("\u{feff}"), Some(ChangeList::Objects(objects)) if objects.is_empty())
        );
        for unexpected in ["", "\r\n", "\u{feff}\r\n"] {
            assert!(parse_designer_diff(unexpected).is_none(), "{unexpected:?}");
        }
        assert!(matches!(
            parse_designer_diff("\u{feff}FullDump\r\n"),
            Some(ChangeList::FullDump)
        ));
        assert!(matches!(
            parse_designer_diff(
                "\u{feff}New: Catalog.Справочник9\r\nModified: CommonModule.ОбщийМодуль1\r\n"
            ),
            Some(ChangeList::Objects(objects)) if objects.len() == 2
        ));
        for unknown in [
            "Deleted: Catalog.X\r\n",
            "FullDump\r\nModified: Catalog.X\r\n",
            "Modified:\r\n",
        ] {
            assert!(parse_designer_diff(unknown).is_none(), "{unknown:?}");
        }
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
