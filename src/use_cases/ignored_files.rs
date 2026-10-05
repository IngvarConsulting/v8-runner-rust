//! Что не должно попадать в гит проекта, и что делать, если попало.
//!
//! Один владелец на оба вопроса: шаблоны, которые `init` и `clone` пишут в
//! `.gitignore`, и отказ `pull`/`push`, нашедших опись версий в индексе.
//!
//! Опись (`ConfigDumpInfo.xml`) — состояние одной базы, а не исходный код: у
//! другой базы она другая и при той же конфигурации. Чужая опись, принесённая
//! `checkout` или `merge`, врёт тихо — инкрементальная выгрузка считает разницу
//! от состояния, которого у этой базы не было. Шаблон в `.gitignore` над уже
//! отслеживаемым файлом бессилен, поэтому одного генератора мало: найдя опись в
//! индексе, раннер останавливается. Чинить индекс сам он не вправе — `git rm
//! --cached` трогает чужой репозиторий, — и называет рецепт.

use std::path::Path;

use crate::platform::git::{check_ignored, tracking_of, Tracking};
use crate::support::error::AppError;

/// Местный слой конфига: адреса баз и пути этой машины.
pub(crate) const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";

/// Имя описи версий, которую платформа пишет в каталог выгрузки.
pub(crate) const VERSION_FILE_NAME: &str = "ConfigDumpInfo.xml";

/// Шаблон `.gitignore` и имя, на котором гит проверяет, покрыт ли он.
struct IgnoredPattern {
    /// Строка, которую генератор дописывает. Без косой черты гит сопоставляет её
    /// на любой глубине — один шаблон закрывает все наборы исходников.
    pattern: &'static str,
    /// Имя, которое шаблон обязан покрывать.
    probe: &'static str,
}

/// Всё, что генератор пишет в `.gitignore` проекта.
const IGNORED_PATTERNS: &[IgnoredPattern] = &[
    IgnoredPattern {
        pattern: LOCAL_CONFIG_FILE_NAME,
        probe: LOCAL_CONFIG_FILE_NAME,
    },
    // Опись версий одной базы.
    IgnoredPattern {
        pattern: VERSION_FILE_NAME,
        probe: VERSION_FILE_NAME,
    },
    // Замок выгрузки рядом с целью (#332): после сбоя он остаётся на диске и не
    // должен уехать в коммит.
    IgnoredPattern {
        pattern: ".dump-*.lock*",
        probe: ".dump-main.lock",
    },
];

/// Дописывает в `.gitignore` недостающие шаблоны проекта.
///
/// Шаблон пропускается, если гит говорит, что имя уже покрыто, — любым файлом
/// игнора, хоть родительским. Без гита решает текст самого файла: шаблон считается
/// записанным, если в нём есть та же строка, с `/` или `**/` впереди. Повторный
/// запуск ничего не дублирует.
pub(crate) fn ensure_project_gitignore(gitignore_path: &Path) -> Result<(), AppError> {
    let dir = gitignore_path.parent().unwrap_or_else(|| Path::new("."));
    let existing = match std::fs::read_to_string(gitignore_path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::Runtime(format!(
                "failed to read gitignore file '{}': {error}",
                gitignore_path.display()
            )))
        }
    };
    let text = existing.as_deref().unwrap_or_default();

    let missing: Vec<&str> = IGNORED_PATTERNS
        .iter()
        .filter(|entry| match check_ignored(&dir.join(entry.probe)) {
            Some(covered) => !covered,
            None => !gitignore_mentions(text, entry.pattern),
        })
        .map(|entry| entry.pattern)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    let mut content = existing.unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for pattern in missing {
        content.push_str(pattern);
        content.push('\n');
    }
    std::fs::write(gitignore_path, content).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write gitignore file '{}': {error}",
            gitignore_path.display()
        ))
    })
}

fn gitignore_mentions(content: &str, pattern: &str) -> bool {
    content.lines().map(str::trim).any(|line| {
        line == pattern
            || line.strip_prefix('/') == Some(pattern)
            || line.strip_prefix("**/") == Some(pattern)
    })
}

/// Отказывает, если опись версий в каталоге `dir` лежит в индексе гита.
///
/// Проверка чистая: она ничего не пишет и потому одинакова в превью и в боевом
/// прогоне. Ответ «неизвестно» — гита нет, каталог вне рабочей копии, гит вернул
/// ошибку — работу не останавливает.
pub(crate) fn refuse_tracked_version_file(dir: &Path) -> Result<(), AppError> {
    match tracking_of(&dir.join(VERSION_FILE_NAME)) {
        Tracking::Tracked(path) => Err(AppError::Validation(tracked_refusal(&path))),
        Tracking::Untracked | Tracking::Unknown(_) => Ok(()),
    }
}

fn tracked_refusal(path: &Path) -> String {
    let shown = shell_word(path);
    // Одной строкой: человеческий вывод — закреплённая форма, и многострочная
    // подробность в нём рассыпается по разным видам строк.
    format!(
        "{VERSION_FILE_NAME} is tracked by git: {shown}; it describes one infobase and must not travel between machines; stop tracking it from the repository root: git rm --cached {shown} && git commit -m \"stop tracking {VERSION_FILE_NAME}\" (keep '{VERSION_FILE_NAME}' in .gitignore)"
    )
}

/// Путь, пригодный для вставки в командную строку как одно слово.
fn shell_word(path: &Path) -> String {
    let text = path.display().to_string();
    if text.chars().any(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn writes_every_pattern_into_a_new_gitignore() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".gitignore");

        ensure_project_gitignore(&path).expect("gitignore");

        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
        );
    }

    #[test]
    fn a_second_run_adds_nothing() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".gitignore");

        ensure_project_gitignore(&path).expect("first");
        ensure_project_gitignore(&path).expect("second");

        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
        );
    }

    #[test]
    fn only_the_missing_patterns_are_appended() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".gitignore");
        fs::write(
            &path,
            "# local\n**/v8project.local.yaml\n/ConfigDumpInfo.xml",
        )
        .expect("seed");

        ensure_project_gitignore(&path).expect("gitignore");

        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "# local\n**/v8project.local.yaml\n/ConfigDumpInfo.xml\n.dump-*.lock*\n"
        );
    }

    #[test]
    fn the_refusal_names_the_path_and_the_recipe_on_one_line() {
        let message = tracked_refusal(Path::new("src/cf/ConfigDumpInfo.xml"));
        assert!(
            message.contains("git rm --cached src/cf/ConfigDumpInfo.xml && git commit"),
            "{message}"
        );
        assert_eq!(message.lines().count(), 1, "{message}");
    }

    #[test]
    fn a_path_with_spaces_stays_one_word() {
        let message = tracked_refusal(Path::new("my src/ConfigDumpInfo.xml"));
        assert!(
            message.contains("git rm --cached \"my src/ConfigDumpInfo.xml\""),
            "{message}"
        );
    }
}
