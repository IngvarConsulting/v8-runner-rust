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

use std::path::{Path, PathBuf};

use tracing::debug;

use crate::platform::git::{ignored_by_worktree_gitignore, tracking_of, Tracking};
use crate::support::error::AppError;

/// Местный слой конфига: адреса баз и пути этой машины.
pub(crate) const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";

/// Имя описи версий, которую платформа пишет в каталог выгрузки.
pub(crate) const VERSION_FILE_NAME: &str = "ConfigDumpInfo.xml";

const GITIGNORE_FILE_NAME: &str = ".gitignore";

/// Подкаталог, на котором проверяется, что шаблон действует на любой глубине, а
/// не только в корне: якорный `/ConfigDumpInfo.xml` наборы в `src/…` не покрывает.
/// Каталога на диске нет и не будет — гит сопоставляет шаблоны по имени.
const NESTED_PROBE_DIR: &str = "v8-runner-probe";

/// Где шаблон обязан действовать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Рядом с конфигом: местный слой лежит только там.
    BesideConfig,
    /// В любом каталоге рабочей копии: опись и замок лежат в каталогах наборов,
    /// а те бывают где угодно.
    AnyDepth,
}

/// Шаблон `.gitignore` и имена, на которых гит проверяет, покрыт ли он.
struct IgnoredPattern {
    /// Строка, которую генератор дописывает. Без косой черты гит сопоставляет её
    /// на любой глубине ниже файла `.gitignore`.
    pattern: &'static str,
    /// Имена, которые шаблон обязан покрывать все до одного.
    probes: &'static [&'static str],
    reach: Reach,
}

/// Всё, что генератор пишет в `.gitignore` проекта.
const IGNORED_PATTERNS: &[IgnoredPattern] = &[
    IgnoredPattern {
        pattern: LOCAL_CONFIG_FILE_NAME,
        probes: &[LOCAL_CONFIG_FILE_NAME],
        reach: Reach::BesideConfig,
    },
    // Опись версий одной базы.
    IgnoredPattern {
        pattern: VERSION_FILE_NAME,
        probes: &[VERSION_FILE_NAME],
        reach: Reach::AnyDepth,
    },
    // Замок выгрузки рядом с целью (#332): после сбоя он остаётся на диске и не
    // должен уехать в коммит. Файла у замка два, и пользовательский `*.lock`
    // покрывает только первый.
    IgnoredPattern {
        pattern: ".dump-*.lock*",
        probes: &[".dump-main.lock", ".dump-main.lock.system"],
        reach: Reach::AnyDepth,
    },
];

/// Файл `.gitignore` проекта и правила, по которым он дописывается.
///
/// Файл один: `.gitignore` каталога проекта — того, где лежат наборы, — в гите и
/// вне его. Не корень рабочей копии: проект бывает подкаталогом чужого
/// репозитория (монорепо, `git init` в домашнем каталоге), и тамошний `.gitignore`
/// раннеру не принадлежит. Не каталог конфига: опись и замок лежат в каталогах
/// наборов, а `.gitignore` в `config/` до `src/…` не дотягивается. Шаблоны без
/// `/` из каталога проекта действуют на любой глубине под ним — и на наборы, и
/// на вложенный конфиг.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectGitignore {
    path: PathBuf,
    project_dir: PathBuf,
    config_dir: PathBuf,
}

impl ProjectGitignore {
    /// Находит файл `.gitignore` проекта из `project_dir`; конфиг лежит в
    /// `config_dir`.
    ///
    /// Ничего не пишет и гита не спрашивает: план `clone` называет этот путь до
    /// того, как что-либо записано. Каталогов может ещё не быть.
    pub(crate) fn locate(project_dir: &Path, config_dir: &Path) -> Self {
        Self {
            path: project_dir.join(GITIGNORE_FILE_NAME),
            project_dir: project_dir.to_path_buf(),
            config_dir: config_dir.to_path_buf(),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Дописывает недостающие шаблоны проекта.
    ///
    /// Шаблон пропускается, если гит говорит, что имя уже покрыто файлом
    /// `.gitignore` внутри рабочей копии — любым: вложенным, этим же или лежащим
    /// выше каталога проекта. Игнор одной машины (`.git/info/exclude`,
    /// `core.excludesFile`) не считается: с репозиторием он не уезжает. Без гита
    /// решает текст самого файла: шаблон считается записанным, если в нём есть та
    /// же строка, с `/` или `**/` впереди. Повторный запуск ничего не дублирует.
    pub(crate) fn ensure(&self) -> Result<(), AppError> {
        let existing = match std::fs::read_to_string(&self.path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Runtime(format!(
                    "failed to read gitignore file '{}': {error}",
                    self.path.display()
                )))
            }
        };
        let text = existing.as_deref().unwrap_or_default();

        let missing: Vec<&str> = IGNORED_PATTERNS
            .iter()
            .filter(|entry| match self.covered_by_git(entry) {
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
        std::fs::write(&self.path, content).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write gitignore file '{}': {error}",
                self.path.display()
            ))
        })
    }

    /// Покрыт ли шаблон по ответу гита; `None` — гит не ответил на пробу: гита
    /// нет, каталог вне рабочей копии или гит вернул ошибку.
    fn covered_by_git(&self, entry: &IgnoredPattern) -> Option<bool> {
        for probe in entry.probes {
            let covered = match entry.reach {
                Reach::BesideConfig => {
                    ignored_by_worktree_gitignore(&self.config_dir, Path::new(probe))?
                }
                Reach::AnyDepth => {
                    // Строкой через `/`, а не `Path::join`: гит ждёт косую черту на
                    // любой платформе, а `join` на Windows вставил бы `\`.
                    let nested = format!("{NESTED_PROBE_DIR}/{probe}");
                    ignored_by_worktree_gitignore(&self.project_dir, Path::new(probe))?
                        && ignored_by_worktree_gitignore(&self.project_dir, Path::new(&nested))?
                }
            };
            if !covered {
                return Some(false);
            }
        }
        Some(true)
    }
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
/// ошибку — работу не останавливает; причина уходит в журнал отладки.
pub(crate) fn refuse_tracked_version_file(dir: &Path) -> Result<(), AppError> {
    let version_file = dir.join(VERSION_FILE_NAME);
    match tracking_of(&version_file) {
        Tracking::Tracked(path) => Err(AppError::Validation(tracked_refusal(&path))),
        Tracking::Untracked => Ok(()),
        Tracking::Unknown(reason) => {
            debug!(
                path = %version_file.display(),
                %reason,
                "git tracking of the version file is unknown; proceeding"
            );
            Ok(())
        }
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

/// Путь, пригодный для вставки в командную строку POSIX-оболочки как одно слово.
///
/// Путь из одних безопасных знаков идёт как есть. Любой другой заключается в
/// одинарные кавычки, внутри которых оболочка ничего не раскрывает; сама кавычка
/// закрывается, экранируется и открывается снова: `'` → `'\''`.
fn shell_word(path: &Path) -> String {
    let text = path.display().to_string();
    let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-');
    if !text.is_empty() && text.chars().all(safe) {
        text
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_git::init_git_repo;
    use std::fs;
    use tempfile::tempdir;

    const ALL_PATTERNS: &str = "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n";

    #[test]
    fn writes_every_pattern_into_a_new_gitignore() {
        let dir = tempdir().expect("tempdir");
        let gitignore = ProjectGitignore::locate(dir.path(), dir.path());
        assert_eq!(gitignore.path(), dir.path().join(".gitignore"));

        gitignore.ensure().expect("gitignore");

        assert_eq!(
            fs::read_to_string(gitignore.path()).expect("read"),
            ALL_PATTERNS
        );
    }

    #[test]
    fn a_second_run_adds_nothing() {
        let dir = tempdir().expect("tempdir");
        let gitignore = ProjectGitignore::locate(dir.path(), dir.path());

        gitignore.ensure().expect("first");
        gitignore.ensure().expect("second");

        assert_eq!(
            fs::read_to_string(gitignore.path()).expect("read"),
            ALL_PATTERNS
        );
    }

    #[test]
    fn a_second_run_in_a_git_worktree_adds_nothing() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        let gitignore = ProjectGitignore::locate(dir.path(), dir.path());

        gitignore.ensure().expect("first");
        gitignore.ensure().expect("second");

        assert_eq!(
            fs::read_to_string(gitignore.path()).expect("read"),
            ALL_PATTERNS
        );
    }

    #[test]
    fn only_the_missing_patterns_are_appended() {
        let dir = tempdir().expect("tempdir");
        let gitignore = ProjectGitignore::locate(dir.path(), dir.path());
        fs::write(
            gitignore.path(),
            "# local\n**/v8project.local.yaml\n/ConfigDumpInfo.xml",
        )
        .expect("seed");

        gitignore.ensure().expect("gitignore");

        assert_eq!(
            fs::read_to_string(gitignore.path()).expect("read"),
            "# local\n**/v8project.local.yaml\n/ConfigDumpInfo.xml\n.dump-*.lock*\n"
        );
    }

    /// Конфиг во вложенном каталоге, наборы — в `src/…`: шаблоны пишутся в
    /// `.gitignore` каталога проекта, иначе до наборов они не дотягиваются.
    #[test]
    fn a_nested_config_ignores_the_version_file_from_the_project_dir() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        let config_dir = dir.path().join("config");
        fs::create_dir_all(&config_dir).expect("config dir");

        let gitignore = ProjectGitignore::locate(dir.path(), &config_dir);
        assert_eq!(gitignore.path(), dir.path().join(".gitignore"));
        gitignore.ensure().expect("gitignore");

        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).expect("read"),
            ALL_PATTERNS
        );
        assert!(!config_dir.join(".gitignore").exists());
        for probe in [
            "src/cf/ConfigDumpInfo.xml",
            "src/cf/.dump-main.lock",
            "src/cf/.dump-main.lock.system",
            "config/v8project.local.yaml",
        ] {
            assert_eq!(
                ignored_by_worktree_gitignore(dir.path(), Path::new(probe)),
                Some(true),
                "{probe}"
            );
        }
    }

    /// Проект — подкаталог чужого репозитория (монорепо, `git init` в домашнем
    /// каталоге): корневой `.gitignore` репозитория не трогается, шаблоны ложатся
    /// в `.gitignore` проекта и покрывают его наборы.
    #[test]
    fn a_project_inside_a_larger_repository_keeps_the_root_gitignore_untouched() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        let root_gitignore = dir.path().join(".gitignore");
        fs::write(&root_gitignore, "target/\n").expect("root gitignore");
        let project_dir = dir.path().join("apps").join("erp");
        fs::create_dir_all(&project_dir).expect("project dir");

        let gitignore = ProjectGitignore::locate(&project_dir, &project_dir);
        assert_eq!(gitignore.path(), project_dir.join(".gitignore"));
        gitignore.ensure().expect("gitignore");

        assert_eq!(
            fs::read_to_string(&root_gitignore).expect("root"),
            "target/\n"
        );
        assert_eq!(
            fs::read_to_string(project_dir.join(".gitignore")).expect("project"),
            ALL_PATTERNS
        );
        for probe in [
            "apps/erp/src/cf/ConfigDumpInfo.xml",
            "apps/erp/src/cf/.dump-main.lock.system",
            "apps/erp/v8project.local.yaml",
        ] {
            assert_eq!(
                ignored_by_worktree_gitignore(dir.path(), Path::new(probe)),
                Some(true),
                "{probe}"
            );
        }
    }

    /// Шаблон, уже записанный в `.gitignore` выше каталога проекта, засчитывается:
    /// покрытие спрашивается у всех файлов игнора рабочей копии.
    #[test]
    fn a_pattern_from_an_enclosing_gitignore_counts() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        fs::write(dir.path().join(".gitignore"), "ConfigDumpInfo.xml\n").expect("root");
        let project_dir = dir.path().join("erp");
        fs::create_dir_all(&project_dir).expect("project dir");

        ProjectGitignore::locate(&project_dir, &project_dir)
            .ensure()
            .expect("gitignore");

        assert_eq!(
            fs::read_to_string(project_dir.join(".gitignore")).expect("project"),
            "v8project.local.yaml\n.dump-*.lock*\n"
        );
    }

    /// Якорный шаблон покрывает только корень: наборам в `src/…` нужен шаблон без
    /// якоря.
    #[test]
    fn an_anchored_pattern_does_not_count_for_source_sets() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        fs::write(
            dir.path().join(".gitignore"),
            "/v8project.local.yaml\n/ConfigDumpInfo.xml\n",
        )
        .expect("seed");

        ProjectGitignore::locate(dir.path(), dir.path())
            .ensure()
            .expect("gitignore");

        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).expect("read"),
            "/v8project.local.yaml\n/ConfigDumpInfo.xml\nConfigDumpInfo.xml\n.dump-*.lock*\n"
        );
    }

    /// Игнор одной машины с репозиторием не уезжает: генератор всё равно пишет
    /// шаблон в `.gitignore`, иначе коллега закоммитит опись.
    #[test]
    fn a_pattern_in_info_exclude_is_still_written() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        fs::write(
            dir.path().join(".git").join("info").join("exclude"),
            "v8project.local.yaml\nConfigDumpInfo.xml\n.dump-*.lock*\n",
        )
        .expect("exclude");

        let gitignore = ProjectGitignore::locate(dir.path(), dir.path());
        gitignore.ensure().expect("gitignore");

        assert_eq!(
            fs::read_to_string(gitignore.path()).expect("read"),
            ALL_PATTERNS
        );
    }

    /// Пользовательский `*.lock` покрывает `.dump-main.lock`, но не его
    /// `.system`-соседа: шаблон замка всё равно нужен.
    #[test]
    fn a_lock_pattern_must_cover_both_lock_files() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        fs::write(
            dir.path().join(".gitignore"),
            "v8project.local.yaml\nConfigDumpInfo.xml\n*.lock\n",
        )
        .expect("seed");

        ProjectGitignore::locate(dir.path(), dir.path())
            .ensure()
            .expect("gitignore");

        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).expect("read"),
            "v8project.local.yaml\nConfigDumpInfo.xml\n*.lock\n.dump-*.lock*\n"
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
            message.contains("git rm --cached 'my src/ConfigDumpInfo.xml'"),
            "{message}"
        );
    }

    /// Оболочка раскрывает не только пробелы: `$`, `*`, `;`, кавычки и прочее
    /// вне безопасного набора идут в одинарных кавычках, а сама кавычка
    /// экранируется.
    #[test]
    fn a_path_with_shell_metacharacters_is_quoted() {
        for (path, expected) in [
            ("src/cf/ConfigDumpInfo.xml", "src/cf/ConfigDumpInfo.xml"),
            (
                "src/$HOME/ConfigDumpInfo.xml",
                "'src/$HOME/ConfigDumpInfo.xml'",
            ),
            ("src/a;b/ConfigDumpInfo.xml", "'src/a;b/ConfigDumpInfo.xml'"),
            ("src/*/ConfigDumpInfo.xml", "'src/*/ConfigDumpInfo.xml'"),
            (
                "src/it's/ConfigDumpInfo.xml",
                r"'src/it'\''s/ConfigDumpInfo.xml'",
            ),
            (
                "src/конф/ConfigDumpInfo.xml",
                "'src/конф/ConfigDumpInfo.xml'",
            ),
        ] {
            assert_eq!(shell_word(Path::new(path)), expected, "{path}");
        }
    }
}
