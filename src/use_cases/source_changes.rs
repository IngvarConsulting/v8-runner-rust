//! Где искать изменения набора: у git или чтением файлов (#575).
//!
//! Набор в рабочей копии git, не в подмодуле и не под `workPath`, берёт изменения у git:
//! сценарий спрашивает `platform::git` один раз на рабочую копию и передаёт разобранный ответ
//! анализу (`change_detection` процессов не запускает). Git отказал или его нет — анализ читает
//! файлы, как прежде, а память хешами git даёт полную загрузку: отказа нет. Отмена завершает
//! git, и ответ — отмена, а не запасной путь (`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`).
//! Правило игнорирования, которое прячет файл набора, — отказ
//! (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::{
    self, ChangeDetectionError, ChangeSource, ContextAnalysis,
};
use crate::change_detection::git_tree::{BlobHashError, GitSource};
use crate::change_detection::scanner;
use crate::domain::git_worktree::{GitWorktree, ScanReason};
use crate::domain::source_set::SourceSetContext;
use crate::platform::git::{self, GitQueryError, Placement};
use crate::support::error::AppError;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::interruption::pending_interruption_error;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

const PLACE: &str = "during source analysis";

/// Как найти изменения одного набора.
enum Choice {
    Git {
        repo: usize,
        root: String,
        work_path: Option<String>,
    },
    Scan(ScanReason),
}

/// Ответ git по рабочим копиям наборов и выбор пути каждого набора.
struct Lookup {
    repos: Vec<GitWorktree>,
    choices: Vec<Choice>,
}

/// Разбирает изменения наборов и отмечает в контексте путь каждого. `designer` — исходники
/// формата Конфигуратора: только они берут изменения у git. `preview` — превью: git
/// спрашивается без необязательных блокировок, индекс не обновляется
/// (`INV.CLI.PREVIEW-LEAVES-NO-TRACE`). Отказ — правило игнорирования, которое прячет файл
/// набора, или отмена.
pub(crate) fn analyze(
    execution: &ExecutionContext,
    contexts: &[SourceSetContext],
    work_path: &Path,
    preview: bool,
    designer: bool,
) -> Result<Vec<ContextAnalysis>, AppError> {
    let mut interrupted = || pending_interruption_error(execution, PLACE).is_some();
    let lookup = if designer {
        lookup(
            contexts,
            work_path,
            Ask::Status {
                optional_locks: !preview,
            },
            &mut interrupted,
        )
        .ok_or_else(|| interruption(execution))?
    } else {
        // Исходники EDT: в базу идёт их копия под `workPath`, и изменения ищет чтение файлов.
        Lookup {
            repos: Vec::new(),
            choices: contexts
                .iter()
                .map(|_| Choice::Scan(ScanReason::EdtFormat))
                .collect(),
        }
    };
    let analyses: Vec<ContextAnalysis> = contexts
        .iter()
        .zip(&lookup.choices)
        .map(|(context, choice)| match choice {
            Choice::Scan(reason) => analyzer::analyze_context_from(
                context,
                work_path,
                ChangeSource::Scan(Some(*reason)),
                &mut interrupted,
            ),
            Choice::Git {
                repo,
                root,
                work_path: work,
            } => {
                let view = &lookup.repos[*repo];
                let mut hasher = blob_hasher(&view.toplevel, !preview);
                analyzer::analyze_context_from(
                    context,
                    work_path,
                    ChangeSource::Git(GitSource {
                        view,
                        root: root.clone(),
                        work_path: work.clone(),
                        hash_blobs: &mut hasher,
                    }),
                    &mut interrupted,
                )
            }
        })
        .collect();
    for analysis in &analyses {
        if let Err(error @ ChangeDetectionError::HiddenByGit { .. }) = &analysis.outcome {
            return Err(hidden_refusal(error));
        }
        if let Some(detection) = analysis.detection {
            execution.note_detection(analysis.context.name(), detection);
        }
    }
    Ok(analyses)
}

fn interruption(execution: &ExecutionContext) -> AppError {
    pending_interruption_error(execution, PLACE).unwrap_or_else(|| {
        AppError::Runtime("source analysis interrupted without a command cancellation".to_owned())
    })
}

fn hidden_refusal(error: &ChangeDetectionError) -> AppError {
    AppError::Refused(Box::new(UseCaseError::new(
        UseCaseErrorKind::SourceSetIgnoredByGit,
        error.to_string(),
    )))
}

/// Отказывает, если правило игнорирования git прячет файл одного из наборов `contexts` — у
/// команд, которые изменений не разбирают, но пишут или читают наборы целиком: полный `pull`,
/// `push --full`, `make`. `git status` не спрашивается.
pub(crate) fn refuse_hidden_sets(
    execution: &ExecutionContext,
    contexts: &[SourceSetContext],
    work_path: &Path,
) -> Result<(), AppError> {
    let mut interrupted = || pending_interruption_error(execution, PLACE).is_some();
    let lookup = lookup(contexts, work_path, Ask::IgnoreRules, &mut interrupted)
        .ok_or_else(|| interruption(execution))?;
    for (context, choice) in contexts.iter().zip(&lookup.choices) {
        if let Choice::Git { repo, root, .. } = choice {
            analyzer::hidden_by_git_rules(context, work_path, &lookup.repos[*repo], root).map_err(
                |error| match error {
                    error @ ChangeDetectionError::HiddenByGit { .. } => hidden_refusal(&error),
                    other => AppError::Runtime(other.to_string()),
                },
            )?;
        }
    }
    Ok(())
}

/// Хеши дерева полной выгрузки `staging` до его публикации в каталог набора `set`: под git —
/// `git hash-object` рабочей копии, куда дерево ляжет, чтобы концы строк считались, как их
/// считает git; вне git — чтением файлов (#575).
pub(crate) fn prepare_full_snapshot(
    set: &SourceSetContext,
    staging: &Path,
) -> Result<analyzer::FullSnapshot, ChangeDetectionError> {
    match toplevel_of(set.path()) {
        Some(toplevel) => {
            let mut hasher = blob_hasher(&toplevel, true);
            analyzer::prepare_full_snapshot_by_git(set, staging, &mut hasher)
        }
        None => analyzer::prepare_full_snapshot(set, staging),
    }
}

/// После опубликованной полной выгрузки обновляет индекс git по корню набора: переписанные
/// файлы git перечитывает здесь, а не в следующей команде (#575). Коммиты, ветки и содержимое
/// индекса не меняются; отказ git — только запись в журнал.
pub(crate) fn refresh_index_after_full_pull(set: &SourceSetContext) {
    let started = std::time::Instant::now();
    let Ok(Placement::Inside { toplevel, .. }) = git::placement_of(set.path(), &mut || false)
    else {
        return;
    };
    let Some(root) = relative_root(set.path(), &toplevel) else {
        return;
    };
    let refreshed = git::refresh_index(&toplevel, &[root], &mut || false);
    tracing::info!(
        source_set = %set.name(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        ok = refreshed.is_ok(),
        "git index refreshed after a full pull"
    );
}

/// Корень рабочей копии git, в которой лежит или ляжет `path`: спрашивается ближайший
/// существующий каталог.
fn toplevel_of(path: &Path) -> Option<PathBuf> {
    let existing = path.ancestors().find(|dir| dir.is_dir())?;
    match git::placement_of(existing, &mut || false) {
        Ok(Placement::Inside { toplevel, .. }) => Some(toplevel),
        _ => None,
    }
}

/// Записывает память набора после полной загрузки: под git — хешами git, иначе чтением файлов.
pub(crate) fn rescan_and_commit_full(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<(), ChangeDetectionError> {
    // Загрузка уже прошла: память пишется и при отложенной отмене.
    let lookup = lookup(
        std::slice::from_ref(context),
        work_path,
        Ask::Status {
            optional_locks: true,
        },
        &mut || false,
    );
    match lookup.as_ref().map(|lookup| (lookup, &lookup.choices[0])) {
        Some((
            lookup,
            Choice::Git {
                repo,
                root,
                work_path: work,
            },
        )) => {
            let view = &lookup.repos[*repo];
            let mut hasher = blob_hasher(&view.toplevel, true);
            analyzer::rescan_and_commit_full_from(
                context,
                work_path,
                ChangeSource::Git(GitSource {
                    view,
                    root: root.clone(),
                    work_path: work.clone(),
                    hash_blobs: &mut hasher,
                }),
            )
        }
        _ => analyzer::rescan_and_commit_full_from(context, work_path, ChangeSource::Scan(None)),
    }
}

/// Хеширует git'ом файлы, которые выгрузка записала в набор `set`, — для памяти хешами git;
/// `f` получает корень набора от корня рабочей копии и сам хешер, или `None` вне git.
pub(crate) fn with_dump_hasher<T>(
    set: &SourceSetContext,
    f: impl FnOnce(Option<(&str, &mut crate::change_detection::git_tree::BlobHasher<'_>)>) -> T,
) -> T {
    let placed = toplevel_of(set.path())
        .and_then(|toplevel| relative_root(set.path(), &toplevel).map(|root| (toplevel, root)));
    match placed {
        Some((toplevel, root)) => {
            let mut hasher = blob_hasher(&toplevel, true);
            f(Some((&root, &mut hasher)))
        }
        None => f(None),
    }
}

fn blob_hasher(
    toplevel: &Path,
    optional_locks: bool,
) -> impl FnMut(&[String], &mut dyn FnMut() -> bool) -> Result<Vec<String>, BlobHashError> + '_ {
    move |paths, interrupted| {
        git::hash_objects(toplevel, paths, optional_locks, interrupted).map_err(|error| match error
        {
            GitQueryError::Interrupted => BlobHashError::Interrupted,
            GitQueryError::Failed(reason) => BlobHashError::Failed(reason),
        })
    }
}

/// Что спросить у git о рабочей копии.
#[derive(Clone, Copy)]
enum Ask {
    /// Только правила игнорирования.
    IgnoreRules,
    /// Индекс и `git status`; `optional_locks` разрешает обновить индекс.
    Status { optional_locks: bool },
}

/// Выбор пути каждого набора и ответ git по их рабочим копиям; `None` — отмена.
fn lookup(
    contexts: &[SourceSetContext],
    work_path: &Path,
    ask: Ask,
    interrupted: &mut dyn FnMut() -> bool,
) -> Option<Lookup> {
    let work_real = work_path.canonicalize().ok();
    struct Repo {
        toplevel: PathBuf,
        info_exclude: PathBuf,
        roots: Vec<String>,
        excluded: Vec<String>,
    }
    let mut repos: Vec<Repo> = Vec::new();
    let mut by_toplevel: HashMap<PathBuf, usize> = HashMap::new();
    let mut choices = Vec::with_capacity(contexts.len());
    for context in contexts {
        // Копия набора EDT лежит под `workPath`: её изменения ищет чтение файлов.
        let under_work_path = context
            .path()
            .canonicalize()
            .ok()
            .zip(work_real.as_ref())
            .is_some_and(|(root, work)| root.starts_with(work));
        if under_work_path {
            choices.push(Choice::Scan(ScanReason::EdtFormat));
            continue;
        }
        let placement = match git::placement_of(context.path(), interrupted) {
            Ok(placement) => placement,
            Err(_) => return None,
        };
        let (toplevel, info_exclude) = match placement {
            Placement::Inside {
                toplevel,
                info_exclude,
            } => (toplevel, info_exclude),
            Placement::Outside(reason) => {
                tracing::debug!(source_set = %context.name(), reason, "the set is not in a git working tree");
                choices.push(Choice::Scan(ScanReason::NoRepository));
                continue;
            }
            Placement::Submodule => {
                choices.push(Choice::Scan(ScanReason::Submodule));
                continue;
            }
            Placement::Failed(reason) => {
                tracing::warn!(source_set = %context.name(), reason, "git failed; the set's files are read");
                choices.push(Choice::Scan(ScanReason::GitFailed));
                continue;
            }
        };
        let Some(root) = relative_root(context.path(), &toplevel) else {
            choices.push(Choice::Scan(ScanReason::GitFailed));
            continue;
        };
        let work = scanner::work_path_inside(context.path(), work_path)
            .and_then(|inside| inside.strip_prefix(context.path()).ok().map(slashed));
        let key = toplevel.canonicalize().unwrap_or_else(|_| toplevel.clone());
        let repo = *by_toplevel.entry(key).or_insert_with(|| {
            repos.push(Repo {
                toplevel,
                info_exclude,
                roots: Vec::new(),
                excluded: Vec::new(),
            });
            repos.len() - 1
        });
        repos[repo].roots.push(root.clone());
        if let Some(work) = &work {
            repos[repo].excluded.push(join(&root, work));
        }
        choices.push(Choice::Git {
            repo,
            root,
            work_path: work,
        });
    }

    let mut views = Vec::with_capacity(repos.len());
    let mut failed = vec![false; repos.len()];
    for (index, repo) in repos.into_iter().enumerate() {
        let answer = match ask {
            Ask::IgnoreRules => git::ignore_view(&repo.toplevel, &repo.info_exclude, interrupted),
            Ask::Status { optional_locks } => git::worktree_under(
                &repo.toplevel,
                &repo.info_exclude,
                &repo.roots,
                &repo.excluded,
                optional_locks,
                interrupted,
            ),
        };
        match answer {
            Ok(view) => views.push(view),
            Err(GitQueryError::Interrupted) => return None,
            Err(GitQueryError::Failed(reason)) => {
                tracing::warn!(toplevel = %repo.toplevel.display(), reason, "git status failed; the sets' files are read");
                failed[index] = true;
                views.push(GitWorktree::default());
            }
        }
    }
    for choice in &mut choices {
        if let Choice::Git { repo, .. } = choice {
            if failed[*repo] {
                *choice = Choice::Scan(ScanReason::GitFailed);
            }
        }
    }
    Some(Lookup {
        repos: views,
        choices,
    })
}

/// Корень набора от корня рабочей копии через `/`; пустая строка — сам корень.
fn relative_root(root: &Path, toplevel: &Path) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let toplevel = toplevel.canonicalize().ok()?;
    root.strip_prefix(&toplevel).ok().map(slashed)
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn join(root: &str, below: &str) -> String {
    if root.is_empty() {
        below.to_owned()
    } else {
        format!("{root}/{below}")
    }
}

#[cfg(test)]
mod tests {
    use super::{analyze, prepare_full_snapshot, refuse_hidden_sets, rescan_and_commit_full};
    use crate::change_detection::analyzer::{
        analyze_context, commit_full_snapshot, AnalysisOutcome, ChangeKind, ContextAnalysis,
    };
    use crate::domain::git_worktree::{ChangeDetection, ScanReason};
    use crate::domain::source_set::SourceSetContext;
    use crate::platform::test_git::{init_git_repo, run_git};
    use crate::support::error::AppError;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::result::UseCaseErrorKind;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Репозиторий с набором в `src/cf` и `workPath` в `.work`, исключённым из git.
    struct Project {
        dir: TempDir,
    }

    impl Project {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            init_git_repo(dir.path());
            run_git(dir.path(), &["config", "core.autocrlf", "false"]);
            fs::write(dir.path().join(".gitignore"), "/.work/\n").expect("gitignore");
            let project = Self { dir };
            for (path, text) in files {
                project.write(path, text);
            }
            run_git(project.dir.path(), &["add", "-A"]);
            run_git(project.dir.path(), &["commit", "-qm", "init"]);
            project
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("src").join("cf")
        }

        fn work(&self) -> PathBuf {
            let work = self.dir.path().join(".work");
            fs::create_dir_all(&work).expect("work");
            work
        }

        fn set(&self) -> SourceSetContext {
            SourceSetContext::new("main", self.root(), "designer-main")
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.root().join(path);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(path, text).expect("write");
        }

        fn remember(&self) {
            rescan_and_commit_full(&self.set(), &self.work()).expect("memory");
        }

        fn analyze(&self) -> ContextAnalysis {
            self.analyze_with(false).expect("analysis")
        }

        fn analyze_with(&self, preview: bool) -> Result<ContextAnalysis, AppError> {
            let execution = ExecutionContext::cli(CommandName::Status);
            let mut analyses = analyze(&execution, &[self.set()], &self.work(), preview, true)?;
            Ok(analyses.remove(0))
        }
    }

    fn changes(analysis: &ContextAnalysis) -> BTreeMap<String, ChangeKind> {
        match &analysis.outcome {
            Ok(AnalysisOutcome::NoChanges) => BTreeMap::new(),
            Ok(AnalysisOutcome::Changes { changes, .. }) => changes
                .iter()
                .map(|change| {
                    let name = change.path.file_name().expect("name").to_string_lossy();
                    (name.into_owned(), change.kind)
                })
                .collect(),
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn remembered(set: &SourceSetContext, work: &Path) -> BTreeMap<String, String> {
        crate::change_detection::hash_storage::HashStorage::new(
            set.storage_path(work).expect("storage"),
        )
        .load_snapshot()
        .expect("snapshot")
        .entries
        .into_iter()
        .map(|(path, state)| (path, state.hash))
        .collect()
    }

    /// Изменение, добавление, удаление и переименование файла называет git; путь — `git`.
    #[test]
    fn git_names_an_edit_an_addition_a_removal_and_a_rename() {
        let project = Project::new(&[
            ("Configuration.xml", "<c/>"),
            ("Kept.bsl", "kept"),
            ("Edited.bsl", "before"),
            ("Removed.bsl", "gone"),
            ("Old.bsl", "moved"),
        ]);
        project.remember();
        assert_eq!(changes(&project.analyze()), BTreeMap::new());

        project.write("Edited.bsl", "after!");
        project.write("Added.bsl", "new");
        fs::remove_file(project.root().join("Removed.bsl")).expect("remove");
        fs::rename(
            project.root().join("Old.bsl"),
            project.root().join("New.bsl"),
        )
        .expect("rename");

        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([
                ("Added.bsl".to_owned(), ChangeKind::Added),
                ("Edited.bsl".to_owned(), ChangeKind::Modified),
                ("New.bsl".to_owned(), ChangeKind::Added),
                ("Old.bsl".to_owned(), ChangeKind::Deleted),
                ("Removed.bsl".to_owned(), ChangeKind::Deleted),
            ])
        );
    }

    /// Правка того же размера со временем, которому git не может верить (не раньше записи
    /// индекса), git перечитывает: она остаётся изменением.
    #[test]
    fn an_edit_of_the_same_size_and_time_is_read_again_when_git_cannot_trust_the_time() {
        let project = Project::new(&[("Module.bsl", "aaaa")]);
        project.remember();
        let module = project.root().join("Module.bsl");
        let index_time = fs::metadata(project.dir.path().join(".git").join("index"))
            .and_then(|meta| meta.modified())
            .expect("index time");
        fs::write(&module, "bbbb").expect("same size");
        fs::File::options()
            .write(true)
            .open(&module)
            .and_then(|file| file.set_modified(index_time))
            .expect("racy time");

        assert_eq!(
            changes(&project.analyze()),
            BTreeMap::from([("Module.bsl".to_owned(), ChangeKind::Modified)])
        );
    }

    /// Правка, которая меняет только концы строк, при `core.autocrlf` — не изменение.
    #[test]
    fn a_line_ending_only_edit_is_not_a_change() {
        let project = Project::new(&[("Module.bsl", "one\ntwo\n")]);
        run_git(project.dir.path(), &["config", "core.autocrlf", "true"]);
        project.remember();
        project.write("Module.bsl", "one\r\ntwo\r\n");

        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// `assume-unchanged` и `skip-worktree` git о правке молчит; раннер читает такие файлы сам.
    #[test]
    fn assume_unchanged_and_skip_worktree_files_are_read_directly() {
        let project = Project::new(&[("Assumed.bsl", "one"), ("Skipped.bsl", "two")]);
        project.remember();
        run_git(
            project.dir.path(),
            &["update-index", "--assume-unchanged", "src/cf/Assumed.bsl"],
        );
        run_git(
            project.dir.path(),
            &["update-index", "--skip-worktree", "src/cf/Skipped.bsl"],
        );
        project.write("Assumed.bsl", "one, edited");
        project.write("Skipped.bsl", "two, edited");

        assert_eq!(
            changes(&project.analyze()),
            BTreeMap::from([
                ("Assumed.bsl".to_owned(), ChangeKind::Modified),
                ("Skipped.bsl".to_owned(), ChangeKind::Modified),
            ])
        );
    }

    /// Набор вне git читается файл за файлом, и ответ называет причину.
    #[test]
    fn a_set_outside_git_is_read_file_by_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("src");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("Module.bsl"), "one").expect("module");
        let set = SourceSetContext::new("main", root.clone(), "designer-main");
        let work = dir.path().join("work");
        rescan_and_commit_full(&set, &work).expect("memory");
        fs::write(root.join("Module.bsl"), "two").expect("edit");

        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, &[set], &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::NoRepository))
        );
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([("Module.bsl".to_owned(), ChangeKind::Modified)])
        );
    }

    /// Отказ git — не отказ команды: набор читается файл за файлом.
    #[test]
    fn a_failing_git_falls_back_to_reading_files() {
        let project = Project::new(&[("Module.bsl", "one")]);
        project.remember();
        project.write("Module.bsl", "two");
        fs::write(project.dir.path().join(".git").join("index"), "broken").expect("break");

        let analysis = project.analyze();
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::GitFailed))
        );
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([("Module.bsl".to_owned(), ChangeKind::Modified)])
        );
    }

    /// Набор в подмодуле читается файл за файлом.
    #[test]
    fn a_set_in_a_submodule_is_read_file_by_file() {
        let inner = Project::new(&[("Module.bsl", "one")]);
        let outer = tempfile::tempdir().expect("outer");
        init_git_repo(outer.path());
        run_git(
            outer.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                inner.dir.path().to_str().expect("utf-8"),
                "sub",
            ],
        );
        let root = outer.path().join("sub").join("src").join("cf");
        let set = SourceSetContext::new("main", root, "designer-main");
        let work = outer.path().join("work");

        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, &[set], &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::Submodule))
        );
    }

    /// Вложенный репозиторий внутри набора git не перечисляет: набор читается сам.
    #[test]
    fn a_nested_repository_inside_the_set_is_read_file_by_file() {
        let project = Project::new(&[("Module.bsl", "one")]);
        project.remember();
        let nested = project.root().join("Nested");
        fs::create_dir_all(&nested).expect("nested");
        init_git_repo(&nested);
        fs::write(nested.join("Inner.bsl"), "inner").expect("inner");

        let analysis = project.analyze();
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::NestedRepository))
        );
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([("Inner.bsl".to_owned(), ChangeKind::Added)])
        );
    }

    /// Превью спрашивает git без необязательных блокировок: индекс не меняется, хотя без превью
    /// тот же вопрос его обновляет.
    #[test]
    fn a_preview_leaves_the_git_index_untouched() {
        let project = Project::new(&[("Module.bsl", "one")]);
        project.remember();
        let index = project.dir.path().join(".git").join("index");
        // То же содержимое и новое время: git перечитает файл и захочет записать индекс.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        project.write("Module.bsl", "one");
        let before = fs::read(&index).expect("index");

        project.analyze_with(true).expect("preview");
        assert_eq!(
            fs::read(&index).expect("index"),
            before,
            "a preview writes no index"
        );

        project.analyze_with(false).expect("analysis");
        assert_ne!(
            fs::read(&index).expect("index"),
            before,
            "the check is not vacuous"
        );
    }

    /// Правило без привязки, которое прячет записанный выгрузкой файл набора, — отказ с правилом,
    /// файлом, строкой и набором; правило с привязкой вне корня — нет.
    #[test]
    fn an_ignore_rule_hiding_a_remembered_file_refuses_the_command() {
        let project = Project::new(&[("Ext/ParentConfigurations.bin", "bin"), ("Module.bsl", "m")]);
        project.remember();
        fs::write(
            project.dir.path().join(".gitignore"),
            "/.work/\n/docs/*.bin\n",
        )
        .expect("rules");
        project
            .analyze_with(false)
            .expect("a rule anchored outside the root hides nothing");

        fs::write(project.dir.path().join(".gitignore"), "/.work/\n*.bin\n").expect("rules");
        let Err(AppError::Refused(refusal)) = project.analyze_with(false) else {
            panic!("expected a refusal");
        };
        assert_eq!(refusal.kind(), UseCaseErrorKind::SourceSetIgnoredByGit);
        assert_eq!(refusal.kind().exit_code(), 2);
        let message = refusal.message();
        assert!(message.contains("'*.bin'"), "{message}");
        assert!(message.contains(".gitignore:2"), "{message}");
        assert!(message.contains("source-set 'main'"), "{message}");
        assert!(
            message.contains("Ext/ParentConfigurations.bin"),
            "{message}"
        );
        assert!(
            message.contains("'/build/' instead of 'build/'"),
            "{message}"
        );

        let execution = ExecutionContext::cli(CommandName::Build);
        assert!(refuse_hidden_sets(&execution, &[project.set()], &project.work()).is_err());
    }

    /// `.gitignore` внутри набора — отказ; правило `ConfigDumpInfo.xml` — нет: этот файл раннер
    /// не читает.
    #[test]
    fn a_gitignore_inside_the_set_refuses_and_the_version_file_rule_does_not() {
        let project = Project::new(&[("Module.bsl", "m"), (".gitignore", "ConfigDumpInfo.xml\n")]);
        project.remember();
        project
            .analyze_with(false)
            .expect("the version file is not read anyway");

        project.write(".gitignore", "*.bsl\n");
        let execution = ExecutionContext::cli(CommandName::Build);
        let Err(AppError::Refused(refusal)) =
            refuse_hidden_sets(&execution, &[project.set()], &project.work())
        else {
            panic!("expected a refusal");
        };
        assert!(
            refusal.message().contains("Module.bsl"),
            "{}",
            refusal.message()
        );
    }

    /// Полный pull под git хеширует дерево выгрузки до публикации git'ом той рабочей копии, куда
    /// оно ляжет: при `core.autocrlf` сразу после публикации изменений нет, хотя байты с CRLF.
    #[test]
    fn a_full_pull_remembers_the_staged_tree_by_git_ids() {
        let project = Project::new(&[("Module.bsl", "one\ntwo\n")]);
        run_git(project.dir.path(), &["config", "core.autocrlf", "true"]);
        let staging = project.work().join("stage");
        fs::create_dir_all(&staging).expect("stage");
        fs::write(staging.join("Module.bsl"), "one\r\ntwo\r\n").expect("dumped");
        let prepared = prepare_full_snapshot(&project.set(), &staging).expect("staged");
        project.write("Module.bsl", "one\r\ntwo\r\n");
        commit_full_snapshot(&project.set(), &project.work(), &prepared).expect("memory");

        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// Сторож повторного появления (#575): git и обход выбирают одни и те же файлы набора и дают
    /// им одни и те же хеши — на дереве, которое задевает каждый предикат обхода: служебные и
    /// порождённые каталоги на любой глубине, опись версий и её временный файл, зонд выгрузки,
    /// `workPath` внутри корня и файл вне индекса.
    #[test]
    fn git_and_scan_snapshots_agree() {
        let project = Project::new(&[
            ("Configuration.xml", "<c/>"),
            ("ConfigDumpInfo.xml", "<v/>"),
            ("Catalogs/Items/Ext/ObjectModule.bsl", "m"),
            ("Catalogs/build/Ext/Inside.bsl", "generated dir at depth"),
            ("target/out.bin", "generated"),
            ("tmp/x.txt", "generated"),
            ("temp/x.txt", "generated"),
            (".gradle/x", "service"),
            (".build/x", "service"),
            (".yaxunit/x", "service"),
            ("Forms/Form.xml", "<f/>"),
            ("Catalogs/Номенклатура.xml", "<n/>"),
        ]);
        project.write("Untracked.bsl", "new");
        project.write("Catalogs/Номенклатура/Ext/ObjectModule.bsl", "новый");
        project.write(
            &format!("{}1", crate::change_detection::scanner::DUMP_START_PROBE),
            "probe",
        );
        project.write(
            &format!(
                "ConfigDumpInfo.xml{}x",
                crate::support::fs::ATOMIC_WRITE_CANDIDATE_SUFFIX
            ),
            "atomic",
        );
        project.write(".work-inside/state", "work");
        let set = project.set();
        let work = project.root().join(".work-inside");

        rescan_and_commit_full(&set, &work).expect("by git");
        let by_git = remembered(&set, &work);
        crate::change_detection::analyzer::rescan_and_commit_full(&set, &work).expect("by scan");
        let by_scan = remembered(&set, &work);
        assert_eq!(by_git, by_scan);
        assert!(by_git.contains_key("Untracked.bsl"));
        assert!(by_git.contains_key("Catalogs/Items/Ext/ObjectModule.bsl"));
        assert!(!by_git
            .keys()
            .any(|path| path.starts_with("Catalogs/build/")));

        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, std::slice::from_ref(&set), &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
        assert!(matches!(
            analyze_context(&set, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    /// Выгрузка по изменившемуся под git запоминает записанное хешами git: при `core.autocrlf`
    /// файл, который она переписала с CRLF, сразу после неё не изменение
    /// (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`).
    #[test]
    fn a_pull_by_changes_under_git_remembers_what_it_wrote_by_git_ids() {
        use crate::change_detection::analyzer::{
            list_after_dump, mark_dump_start, observe_dump_writes, watch_before_dump,
        };

        let project = Project::new(&[("Module.bsl", "one\ntwo\n")]);
        run_git(project.dir.path(), &["config", "core.autocrlf", "true"]);
        let set = project
            .set()
            .with_infobase_memory("ib", "file:/ib; source=src/cf".to_owned());
        let work = project.work();
        rescan_and_commit_full(&set, &work).expect("memory");

        let start = mark_dump_start(&project.root()).expect("start");
        let watch = watch_before_dump(&set, &work, start).expect("memory of this base");
        std::thread::sleep(std::time::Duration::from_millis(50));
        project.write("Module.bsl", "one\r\ntwo\r\nthree\r\n");
        let listed = list_after_dump(&set, &work, &|| false).expect("listed");
        super::with_dump_hasher(&set, |blobs| {
            assert!(blobs.is_some(), "the set lies in git");
            observe_dump_writes(&set, &work, &watch, None, listed, blobs, &|| false)
        })
        .expect("observed")
        .commit(&set, &work)
        .expect("remembered");

        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, std::slice::from_ref(&set), &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// После опубликованной полной выгрузки раннер обновляет индекс git по корню набора: файлы с
    /// новым временем git перечитывает здесь, и индекс записан заново.
    #[test]
    fn a_full_pull_refreshes_the_git_index_of_the_set() {
        let project = Project::new(&[("Module.bsl", "one")]);
        let index = project.dir.path().join(".git").join("index");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        project.write("Module.bsl", "one");
        let before = fs::read(&index).expect("index");

        super::refresh_index_after_full_pull(&project.set());
        assert_ne!(fs::read(&index).expect("index"), before);
    }
}
