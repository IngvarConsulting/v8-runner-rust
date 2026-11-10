//! Каталоги раннера внутри корня исходников: `workPath` и `localPath`. Единственный владелец
//! ответа, что под корнем набора не исходник, а состояние раннера
//! (`INV.CHANGE-DETECTION.RUNNER-DIRECTORIES-ARE-NOT-SOURCES`): его спрашивают сканер,
//! анализатор, частичная загрузка, копия для агента и защита полной выгрузки от перезаписи.

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::support::path::nearest_existing_canonical_path;

/// Каталоги раннера одного запуска. `local` нет у контекста, собранного без настроек: тогда
/// `localPath` по имени держит вторая защита — `.local` среди пропускаемых каталогов сканера.
#[derive(Debug, Clone, Copy)]
pub struct RunnerDirs<'a> {
    work: &'a Path,
    local: Option<&'a Path>,
}

impl<'a> RunnerDirs<'a> {
    /// Каталоги раннера проекта.
    pub fn of(config: &'a AppConfig) -> Self {
        Self::new(&config.work_path, Some(&config.local_path))
    }

    pub fn new(work: &'a Path, local: Option<&'a Path>) -> Self {
        Self { work, local }
    }
}

/// Каталоги раннера под корнем `root`, названные так, как их называет обход этого корня. Набор
/// в корне проекта держит оба, и имя у них может быть любым. Сам корень сюда не входит.
/// Ещё не созданный каталог называется по месту, которое займёт.
pub fn runner_dirs_inside(root: &Path, dirs: RunnerDirs<'_>) -> Vec<PathBuf> {
    let Ok(root_real) = root.canonicalize() else {
        return Vec::new();
    };
    std::iter::once(dirs.work)
        .chain(dirs.local)
        .filter_map(|dir| {
            // Каталога может ещё не быть: его место под корнем то же.
            let real = nearest_existing_canonical_path(dir).ok()?;
            let relative = real.strip_prefix(&root_real).ok()?;
            (!relative.as_os_str().is_empty()).then(|| root.join(relative))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_runner_directories_inside_the_root_are_named_whatever_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let work = root.join("service-area");
        let local = root.join("personal");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        assert_eq!(
            runner_dirs_inside(&root, RunnerDirs::new(&work, Some(&local))),
            vec![work.clone(), local.clone()]
        );
        assert!(
            runner_dirs_inside(&root.join("service-area"), RunnerDirs::new(&root, None)).is_empty()
        );
        assert!(runner_dirs_inside(&root, RunnerDirs::new(&root, Some(dir.path()))).is_empty());
    }

    /// Защита от повторения: каталоги раннера под корнем называет только этот модуль. Прямой
    /// вызов прежнего `work_path_inside` забывал `localPath` (#596). Защита сравнивает по
    /// имени: копию под другим именем она не увидит, такую ловит ревью.
    #[test]
    fn runner_directories_are_named_only_here() {
        fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(path);
                }
            }
        }
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let forbidden = ["work_path", "_inside("].concat();
        let offenders: Vec<String> = files
            .iter()
            .filter(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .contains(forbidden.as_str())
            })
            .map(|path| path.display().to_string())
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}
