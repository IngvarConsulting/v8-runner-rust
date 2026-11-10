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

/// Каталог раннера, который совпадает с `path` или содержит его.
pub fn runner_dir_holding<'a>(path: &Path, dirs: RunnerDirs<'a>) -> Option<&'a Path> {
    let real = nearest_existing_canonical_path(path).ok()?;
    std::iter::once(dirs.work)
        .chain(dirs.local)
        .find(|dir| nearest_existing_canonical_path(dir).is_ok_and(|dir| real.starts_with(dir)))
}

/// Пересекается ли `path` с каталогом раннера в любую сторону: совпадает, лежит внутри или
/// содержит его. Так проверяется каталог, который команда заменит или отдаст наружу.
pub fn overlaps_a_runner_dir(path: &Path, dirs: RunnerDirs<'_>) -> bool {
    runner_dir_holding(path, dirs).is_some() || !runner_dirs_inside(path, dirs).is_empty()
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

    #[test]
    fn a_path_overlaps_a_runner_directory_in_either_direction() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        let local = dir.path().join(".local");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        let dirs = RunnerDirs::new(&work, Some(&local));
        assert!(overlaps_a_runner_dir(&local, dirs));
        assert!(overlaps_a_runner_dir(&local.join("stage"), dirs));
        assert!(overlaps_a_runner_dir(dir.path(), dirs));
        assert_eq!(
            runner_dir_holding(&local.join("x"), dirs),
            Some(local.as_path())
        );
        assert!(!overlaps_a_runner_dir(&dir.path().join("stage"), dirs));
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
        // Прямые вопросы о вложенности в `workPath` мимо этого модуля: прежний
        // `work_path_inside` и сравнения канонических путей с ним. Строки собраны из частей,
        // чтобы сторож не нашёл сам себя.
        let forbidden = [
            ["work_path", "_inside("].concat(),
            ["starts_with(&", "work)"].concat(),
            ["work.", "starts_with("].concat(),
            ["starts_with(&", "canonical_work_path)"].concat(),
        ];
        let offenders: Vec<String> = files
            .iter()
            .filter(|path| !path.ends_with("change_detection/runner_dirs.rs"))
            .filter_map(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                forbidden
                    .iter()
                    .find(|shape| text.contains(shape.as_str()))
                    .map(|shape| format!("{}: {shape}", path.display()))
            })
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}
