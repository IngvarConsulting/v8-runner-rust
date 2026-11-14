#[cfg(test)]
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::domain::issue::{Issue, IssueSeverity, ModuleIssue, ObjectIssue};

static MODULE_ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\{(?:(?P<extension>[[:word:]А-Яа-яЁё_]+)\s)?(?P<path>[^}]+)\((?P<line>\d+),(?P<column>\d+)\)}:\s*(?P<message>.+)$")
        .expect("module issue regex")
});
static MODULE_CONTEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\{\d+\}\s*:").expect("module context regex"));

pub fn parse(content: &str) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if is_success_message(trimmed) {
            continue;
        }

        if let Some(issue) = parse_module_issue(trimmed) {
            if lines
                .peek()
                .is_some_and(|next_line| is_module_context(next_line.trim()))
            {
                let _ = lines.next();
            }
            issues.push(issue);
            continue;
        }

        if let Some(issue) = parse_object_issue(trimmed) {
            issues.push(issue);
        }
    }

    issues
}

#[cfg(test)]
pub fn parse_path(path: &Path) -> std::io::Result<Vec<Issue>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    Ok(parse(&std::fs::read_to_string(path)?))
}

fn parse_module_issue(line: &str) -> Option<Issue> {
    let captures = MODULE_ISSUE_RE.captures(line)?;
    let extension = captures
        .name("extension")
        .map(|m| format!("{} ", m.as_str()));
    let path = format!(
        "{}{}",
        extension.as_deref().unwrap_or(""),
        captures.name("path")?.as_str()
    );
    let line_num = captures.name("line")?.as_str().parse().ok()?;
    let column = captures.name("column")?.as_str().parse().ok()?;
    let message = captures.name("message")?.as_str().trim().to_owned();

    Some(Issue::Module(ModuleIssue {
        path,
        line: Some(line_num),
        column: Some(column),
        severity: classify_severity(&message),
        message,
    }))
}

fn parse_object_issue(line: &str) -> Option<Issue> {
    let (first, tail) = line.split_once(' ')?;
    let tail = tail.trim();

    // The handler checks can prefix the metadata path with an extension name.
    let extension_issue = tail.split_once(' ').filter(|(path, message)| {
        is_handler_check_message(message.trim_start())
            && is_identifier(first)
            && is_metadata_path(path)
    });
    let (object, message, severity) = if let Some((path, message)) = extension_issue {
        (
            format!("{first} {path}"),
            message.trim_start(),
            IssueSeverity::Error,
        )
    } else if is_handler_check_message(tail) && is_metadata_path(first) {
        (first.to_owned(), tail, IssueSeverity::Error)
    } else if contains_issue_marker(tail) {
        (first.to_owned(), tail, classify_severity(tail))
    } else {
        return None;
    };

    Some(Issue::Object(ObjectIssue {
        object,
        message: message.to_owned(),
        severity,
    }))
}

fn is_metadata_path(path: &str) -> bool {
    path.contains('.') && path.split('.').all(is_identifier)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn is_handler_check_message(message: &str) -> bool {
    message.starts_with("Пустой обработчик:")
        || message.starts_with("Не обнаружено ссылок на функцию:")
        || message.starts_with("Отсутствует обработчик:")
}

fn classify_severity(message: &str) -> IssueSeverity {
    let lower = message.to_lowercase();
    if contains_warning_marker(&lower) {
        IssueSeverity::Warning
    } else {
        IssueSeverity::Error
    }
}

fn is_module_context(line: &str) -> bool {
    MODULE_CONTEXT_RE.is_match(line)
}

fn contains_issue_marker(message: &str) -> bool {
    let lower = message.to_lowercase();
    contains_warning_marker(&lower)
        || lower.contains("error")
        || lower.contains("ошиб")
        || lower.contains("fatal")
        || lower.contains("неразрешим")
}

fn is_success_message(line: &str) -> bool {
    let english = line.strip_suffix('.').unwrap_or(line);
    english.eq_ignore_ascii_case("No errors found")
        || line.to_lowercase().contains("ошибок не обнаружено")
}

fn contains_warning_marker(lower_message: &str) -> bool {
    lower_message.contains("warning")
        || lower_message.contains("warn:")
        || lower_message.contains("предупреждение")
        || lower_message.contains("warning:")
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_path};
    use crate::domain::issue::{Issue, IssueSeverity, ObjectIssue};
    use tempfile::tempdir;

    const DESIGNER_VALIDATION_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parsers/designer_validation.log"
    ));

    #[test]
    fn parses_realistic_fixture_sample() {
        let issues = parse(DESIGNER_VALIDATION_FIXTURE);

        assert_eq!(issues.len(), 4);
        match &issues[0] {
            Issue::Module(issue) => {
                assert_eq!(issue.path, "CommonModules.TestModule");
                assert_eq!(issue.line, Some(12));
                assert_eq!(issue.column, Some(3));
                assert_eq!(issue.severity, IssueSeverity::Error);
            }
            _ => panic!("expected module issue"),
        }

        match &issues[1] {
            Issue::Object(issue) => assert_eq!(issue.object, "Catalogs.Items"),
            _ => panic!("expected object issue"),
        }

        match &issues[2] {
            Issue::Object(issue) => {
                assert_eq!(issue.object, "Справочники.Номенклатура");
                assert_eq!(issue.severity, IssueSeverity::Warning);
            }
            _ => panic!("expected warning object issue"),
        }

        match &issues[3] {
            Issue::Object(issue) => {
                assert_eq!(issue.object, "ОбщаяФорма.НастройкиРегистрации.Справка");
                assert_eq!(issue.severity, IssueSeverity::Error);
            }
            _ => panic!("expected unresolvable reference issue"),
        }
    }

    #[test]
    fn parses_object_issue() {
        let issues = parse("Catalogs.Items Ошибка проверки объекта");

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            Issue::Object(issue) => {
                assert_eq!(issue.object, "Catalogs.Items");
                assert_eq!(issue.severity, IssueSeverity::Error);
            }
            _ => panic!("expected object issue"),
        }
    }

    #[test]
    fn parses_cyrillic_names() {
        let issues = parse("Справочники.Номенклатура Предупреждение: неиспользуемый реквизит");

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            Issue::Object(issue) => {
                assert_eq!(issue.object, "Справочники.Номенклатура");
                assert_eq!(issue.severity, IssueSeverity::Warning);
            }
            _ => panic!("expected object issue"),
        }
    }

    #[test]
    fn keeps_issue_after_module_issue_when_no_context_line_exists() {
        let issues =
            parse("{CommonModules.A(1,1)}: Ошибка компиляции\nКонфигурация Ошибка проверки");

        assert_eq!(issues.len(), 2);
        match &issues[1] {
            Issue::Object(issue) => assert_eq!(issue.object, "Конфигурация"),
            _ => panic!("expected object issue"),
        }
    }

    #[test]
    fn parses_root_level_object_issue_without_dot() {
        let issues = parse("Конфигурация Ошибка проверки объекта");

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            Issue::Object(issue) => assert_eq!(issue.object, "Конфигурация"),
            _ => panic!("expected object issue"),
        }
    }

    #[test]
    fn ignores_noise_only_log() {
        let issues = parse("some random text\n\nanother line");

        assert!(issues.is_empty());
    }

    #[test]
    fn parses_empty_log() {
        let issues = parse("");

        assert!(issues.is_empty());
    }

    #[test]
    fn ignores_success_message() {
        let issues = parse("Синтаксических ошибок не обнаружено!");

        assert!(issues.is_empty());
    }

    #[test]
    fn parses_empty_handler_issue() {
        assert_eq!(
            parse(
                r#"ОбщаяКоманда.ОбщаяКоманда1.МодульКоманды Пустой обработчик: "ОбработкаКоманды""#
            ),
            vec![Issue::Object(ObjectIssue {
                object: "ОбщаяКоманда.ОбщаяКоманда1.МодульКоманды".to_owned(),
                message: r#"Пустой обработчик: "ОбработкаКоманды""#.to_owned(),
                severity: IssueSeverity::Error,
            })]
        );
    }

    #[test]
    fn handler_name_does_not_change_diagnostic_severity() {
        let issues = parse(r#"ОбщаяКоманда.Тест.МодульКоманды Пустой обработчик: "ShowWarning""#);

        assert!(
            matches!(issues.as_slice(), [Issue::Object(issue)] if issue.severity == IssueSeverity::Error)
        );
    }

    #[test]
    fn parses_extension_handler_and_reference_issues() {
        let log = r#"client_mcp ОбщийМодуль.Мсп_ЗадачиКлиент.Модуль Не обнаружено ссылок на функцию: "ВыполнитьОповещение"
YAXUNIT Обработка.ЮТПомощникДляСозданияТестовыхДанных.Форма.Форма.Форма Отсутствует обработчик:  СнятьВсеФлажки "СнятьВсеФлажки""#;

        assert_eq!(
            parse(log),
            vec![
                Issue::Object(ObjectIssue {
                    object: "client_mcp ОбщийМодуль.Мсп_ЗадачиКлиент.Модуль".to_owned(),
                    message: r#"Не обнаружено ссылок на функцию: "ВыполнитьОповещение""#.to_owned(),
                    severity: IssueSeverity::Error,
                }),
                Issue::Object(ObjectIssue {
                    object:
                        "YAXUNIT Обработка.ЮТПомощникДляСозданияТестовыхДанных.Форма.Форма.Форма"
                            .to_owned(),
                    message: r#"Отсутствует обработчик:  СнятьВсеФлажки "СнятьВсеФлажки""#
                        .to_owned(),
                    severity: IssueSeverity::Error,
                }),
            ]
        );
    }

    #[test]
    fn ignores_handler_markers_in_service_text() {
        let log = "Проверка: Пустой обработчик: образец\n\
                   Проверка: строка Отсутствует обработчик: образец\n\
                   ОбщаяКоманда.Тест Проверка: Пустой обработчик: образец\n\
                   1. Пустой обработчик: образец\n\
                   https://example.com Пустой обработчик: образец\n\
                   Проверка 12.03 Отсутствует обработчик: образец";

        assert!(parse(log).is_empty());
    }

    #[test]
    fn ignores_english_success_messages() {
        assert!(parse("No errors found\r\n  No errors found.  \r\nNO ERRORS FOUND").is_empty());
    }

    #[test]
    fn preserves_english_diagnostics_containing_the_success_phrase() {
        let log = "No errors found\n\
                   Catalogs.Items Error: invalid reference\n\
                   Catalogs.Other Error: unexpected result 'No errors found'\n\
                   {CommonModules.TestModule(12,3)}: Error: unexpected result 'No errors found'";
        let issues = parse(log);

        assert!(matches!(
            issues.as_slice(),
            [Issue::Object(first), Issue::Object(second), Issue::Module(third)]
                if first.object == "Catalogs.Items"
                    && first.message == "Error: invalid reference"
                    && first.severity == IssueSeverity::Error
                    && second.message == "Error: unexpected result 'No errors found'"
                    && third.message == second.message
        ));
    }

    #[test]
    fn parse_path_returns_empty_for_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("missing.log");

        let issues = parse_path(&path).expect("parse path");

        assert!(issues.is_empty());
    }

    #[test]
    fn parses_unresolvable_references_issue() {
        let issues = parse(
            "ОбщаяФорма.НастройкиРегистрации.Справка Неразрешимые ссылки на объекты метаданных (1)",
        );

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            Issue::Object(issue) => {
                assert_eq!(issue.object, "ОбщаяФорма.НастройкиРегистрации.Справка");
                assert_eq!(issue.severity, IssueSeverity::Error);
            }
            _ => panic!("expected object issue"),
        }
    }
}
