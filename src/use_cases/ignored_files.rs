//! Что не должно попадать в гит проекта: шаблоны, которые `init` и `clone` пишут в
//! `.gitignore` — местный слой конфига и замок выгрузки.
//!
//! Опись версий (`ConfigDumpInfo.xml`) раннер в каталоги наборов не кладёт (#646): она живёт
//! в его памяти ([`crate::use_cases::version_file`]), и игнорировать её в проекте незачем.

use std::path::{Path, PathBuf};

use crate::platform::git::ignored_by_worktree_gitignore;
use crate::support::error::AppError;

/// Местный слой конфига: адреса баз и пути этой машины.
pub(crate) const LOCAL_CONFIG_FILE_NAME: &str = "v8project.local.yaml";

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

    /// Дописывает недостающие шаблоны проекта: [`Self::planned_content`] и запись.
    pub(crate) fn ensure(&self) -> Result<(), AppError> {
        let Some(content) = self.planned_content()? else {
            return Ok(());
        };
        std::fs::write(&self.path, content).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write gitignore file '{}': {error}",
                self.path.display()
            ))
        })
    }

    /// Содержимое `.gitignore` с недостающими шаблонами; `None` — дописывать нечего.
    /// Файл только читается: записать — дело вызывающего, когда все отказы позади.
    ///
    /// Шаблон пропускается, если гит говорит, что имя уже покрыто файлом
    /// `.gitignore` внутри рабочей копии — любым: вложенным, этим же или лежащим
    /// выше каталога проекта. Игнор одной машины (`.git/info/exclude`,
    /// `core.excludesFile`) не считается: с репозиторием он не уезжает. Без гита
    /// решает текст самого файла: шаблон считается записанным, если в нём есть та
    /// же строка, с `/` или `**/` впереди. Повторный запуск ничего не дублирует.
    pub(crate) fn planned_content(&self) -> Result<Option<String>, AppError> {
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
            return Ok(None);
        }

        let mut content = existing.unwrap_or_default();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for pattern in missing {
            content.push_str(pattern);
            content.push('\n');
        }
        Ok(Some(content))
    }

    /// Покрыт ли шаблон по ответу гита; `None` — гит не ответил на пробу: гита
    /// нет, каталог вне рабочей копии или гит вернул ошибку.
    fn covered_by_git(&self, entry: &IgnoredPattern) -> Option<bool> {
        for probe in entry.probes {
            let covered = match entry.reach {
                Reach::BesideConfig => match self.config_dir.strip_prefix(&self.project_dir) {
                    // Из каталога проекта: каталога конфига до записи может ещё не быть,
                    // а `git -C` в несуществующем каталоге не отвечает вовсе.
                    Ok(relative) => {
                        let beside = relative
                            .components()
                            .map(|part| part.as_os_str().to_string_lossy())
                            .chain(std::iter::once((*probe).into()))
                            .collect::<Vec<_>>()
                            .join("/");
                        ignored_by_worktree_gitignore(&self.project_dir, Path::new(&beside))?
                    }
                    Err(_) => ignored_by_worktree_gitignore(&self.config_dir, Path::new(probe))?,
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_git::init_git_repo;
    use std::fs;
    use tempfile::tempdir;

    /// Опись версий в игнор не пишется: в каталоги наборов раннер её не кладёт (#646).
    const ALL_PATTERNS: &str = "v8project.local.yaml\n.dump-*.lock*\n";

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
    fn a_nested_config_ignores_the_lock_from_the_project_dir() {
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

    /// Каталога конфига до записи ещё нет: покрытие местного слоя гит проверяет из каталога
    /// проекта, и шаблон-маска уже покрывает его — дублей не пишется.
    #[test]
    fn a_config_directory_not_yet_created_is_probed_from_the_project_dir() {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path());
        fs::write(dir.path().join(".gitignore"), "*.local.yaml\n").expect("seed");
        let config_dir = dir.path().join("config");

        let gitignore = ProjectGitignore::locate(dir.path(), &config_dir);

        assert_eq!(
            gitignore.planned_content().expect("plan").as_deref(),
            Some("*.local.yaml\n.dump-*.lock*\n")
        );
        assert!(!config_dir.exists(), "planning creates nothing");
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
        fs::write(dir.path().join(".gitignore"), ".dump-*.lock*\n").expect("root");
        let project_dir = dir.path().join("erp");
        fs::create_dir_all(&project_dir).expect("project dir");

        ProjectGitignore::locate(&project_dir, &project_dir)
            .ensure()
            .expect("gitignore");

        assert_eq!(
            fs::read_to_string(project_dir.join(".gitignore")).expect("project"),
            "v8project.local.yaml\n"
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
            "/v8project.local.yaml\n/.dump-*.lock*\n",
        )
        .expect("seed");

        ProjectGitignore::locate(dir.path(), dir.path())
            .ensure()
            .expect("gitignore");

        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).expect("read"),
            "/v8project.local.yaml\n/.dump-*.lock*\n.dump-*.lock*\n"
        );
    }

    /// Игнор одной машины с репозиторием не уезжает: генератор всё равно пишет
    /// шаблон в `.gitignore`, иначе коллега закоммитит местный слой.
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
}
