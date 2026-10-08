//! Прогноз режима выгрузки по изменившемуся: что платформа сделает с `-update` по этому файлу
//! версий (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
//!
//! Конфигуратор и агент отвечают списком `-getChanges` (`--get-changes`), `ibcmd` — ответом
//! `config export status`. Словари замерены (`references/1c/confirmed-runtime-measurements.md`,
//! разделы о прогнозе режима): от языка интерфейса они не зависят. Решение берётся по ключу
//! строки, а не по прозе; всё, что словарю не принадлежит, — прогноз «неизвестен».

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
    let text = String::from_utf8_lossy(bytes);
    let lines = entries(&text);
    if lines == ["FullDump"] {
        return DumpForecast::Full;
    }
    let changes = lines.iter().all(|line| {
        matches!(
            line.split_once(' '),
            Some(("New:" | "Modified:", object)) if !object.is_empty()
        )
    });
    if changes {
        DumpForecast::Changes
    } else {
        DumpForecast::Unknown
    }
}

/// Ответ `ibcmd config export status` в полной форме: `modified: all` — полная; только
/// `added: <объект>` и `modified: <объект>` или пусто — изменившееся.
pub fn read_export_status(text: &str) -> DumpForecast {
    let lines = entries(text);
    if lines == ["modified: all"] {
        return DumpForecast::Full;
    }
    let changes = lines.iter().all(|line| {
        matches!(
            line.split_once(": "),
            Some(("added" | "modified", object)) if !object.is_empty() && object != "all"
        )
    });
    if changes {
        DumpForecast::Changes
    } else {
        DumpForecast::Unknown
    }
}

/// Непустые строки ответа без BOM и концов строк обеих платформ.
fn entries(text: &str) -> Vec<&str> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .collect()
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
