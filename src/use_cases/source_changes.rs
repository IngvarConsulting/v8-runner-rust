//! Где искать изменения набора: у git или чтением файлов (#575).
//!
//! Набор в рабочей копии git, не в подмодуле и не под `workPath`, берёт изменения у git:
//! сценарий спрашивает `platform::git` один раз на рабочую копию и передаёт разобранный ответ
//! анализу (`change_detection` процессов не запускает). Git отказал или его нет — анализ читает
//! файлы сам: отказа нет. Хеш файла — тот, что сохранил бы `git add`; вне git — хеш байтов, и
//! память помнит, каким хешером записана. Отмена завершает git — а `git status`, которому
//! разрешено обновить индекс, доводит до конца, — и ответ — отмена, а не запасной путь
//! (`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`). Правило игнорирования, которое прячет файл
//! набора, — отказ (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::change_detection::analyzer::{
    self, ChangeDetectionError, ChangeSource, ContextAnalysis,
};
use crate::change_detection::git_tree::{BlobHashError, BlobHasher, GitSource};
use crate::domain::git_worktree::{GitIndexEntry, GitWorktree, ScanReason};
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
        /// Каталоги раннера под корнем набора, от корня набора.
        runner_dirs: Vec<String>,
    },
    Scan(ScanReason),
}

/// Ответ git по рабочим копиям наборов и выбор пути каждого набора.
struct Lookup {
    repos: Vec<GitWorktree>,
    choices: Vec<Choice>,
}

impl Lookup {
    /// Источник изменений набора `index` и хешер для него.
    fn with_source<T>(&self, index: usize, f: impl FnOnce(ChangeSource<'_, '_>) -> T) -> T {
        match &self.choices[index] {
            Choice::Scan(reason) => f(ChangeSource::Scan(Some(*reason))),
            Choice::Git {
                repo,
                root,
                runner_dirs,
            } => {
                let view = &self.repos[*repo];
                let mut hasher = blob_hasher(&view.toplevel, &view.index);
                f(ChangeSource::Git(GitSource {
                    view,
                    root: root.clone(),
                    runner_dirs: runner_dirs.clone(),
                    hash_blobs: &mut hasher,
                }))
            }
        }
    }
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
        .enumerate()
        .map(|(index, context)| {
            lookup.with_source(index, |source| {
                analyzer::analyze_context_from(context, work_path, source, &mut interrupted)
            })
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
/// `push --full`, `make`. `git status` не спрашивается, только настройки и список `.gitignore`.
///
/// Отмену проверка не замечает: она идёт до первой безопасной точки команды, и отмену,
/// пришедшую раньше, называет эта точка, как и до #575
/// (`INV.USE-CASES.MAKE-STOPS-AT-THE-SAFE-POINTS-OF-ITS-BUILD`).
pub(crate) fn refuse_hidden_sets(
    contexts: &[SourceSetContext],
    work_path: &Path,
) -> Result<(), AppError> {
    let Some(lookup) = lookup(contexts, work_path, Ask::IgnoreRules, &mut || false) else {
        return Ok(());
    };
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

/// Хеши дерева полной выгрузки `staging` до его публикации: чтением файлов, без git.
pub(crate) fn prepare_full_snapshot(
    set: &SourceSetContext,
    staging: &Path,
) -> Result<analyzer::FullSnapshot, ChangeDetectionError> {
    analyzer::prepare_full_snapshot(set, staging)
}

/// Память полной выгрузки, уже опубликованной в каталог набора `set` (#575). Под git
/// `git status` по корню набора обновляет индекс — переписанные файлы git перечитывает здесь,
/// а не в следующей команде, — и хеши опубликованного дерева берутся такими, какими их
/// сохранил бы `git add`: запоминается файл, время которого то же, что в промежуточной копии,
/// так что правка после публикации остаётся изменением. Вне git, или если git отказал, —
/// `prepared`, хеши байтов промежуточной копии. Время шага пишется в журнал.
pub(crate) fn full_pull_memory(
    set: &SourceSetContext,
    work_path: &Path,
    prepared: analyzer::FullSnapshot,
) -> Result<analyzer::FullSnapshot, ChangeDetectionError> {
    let started = std::time::Instant::now();
    // Публикация уже прошла: шаг доводится до конца и при отложенной отмене.
    let lookup = lookup(
        std::slice::from_ref(set),
        work_path,
        Ask::Status {
            optional_locks: true,
        },
        &mut || false,
    );
    let by_git = match &lookup {
        Some(lookup) => lookup.with_source(0, |source| match source {
            ChangeSource::Git(git) => analyzer::full_snapshot_by_git(set, &prepared, git),
            ChangeSource::Scan(_) => Ok(None),
        })?,
        None => None,
    };
    tracing::info!(
        source_set = %set.name(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        by_git = by_git.is_some(),
        "git index refreshed and the memory of a full pull taken"
    );
    Ok(by_git.unwrap_or(prepared))
}

/// Записывает память набора после полной загрузки: под git — хешами git, иначе чтением файлов.
pub(crate) fn rescan_and_commit_full(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<(), ChangeDetectionError> {
    // Загрузка уже прошла: память пишется и при отложенной отмене.
    match lookup(
        std::slice::from_ref(context),
        work_path,
        Ask::Status {
            optional_locks: true,
        },
        &mut || false,
    ) {
        Some(lookup) => lookup.with_source(0, |source| {
            analyzer::rescan_and_commit_full_from(context, work_path, source)
        }),
        None => analyzer::rescan_and_commit_full_from(context, work_path, ChangeSource::Scan(None)),
    }
}

/// Хеши дерева набора для памяти базы, созданной раннером: под git — какими их сохранил бы
/// `git add`, иначе хеши байтов. Одна память — один хешер (#575).
pub(crate) fn prepare_source_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<analyzer::FullSnapshot, ChangeDetectionError> {
    match lookup(
        std::slice::from_ref(context),
        work_path,
        Ask::Status {
            optional_locks: true,
        },
        &mut || false,
    ) {
        Some(lookup) => lookup.with_source(0, |source| {
            analyzer::prepare_source_snapshot_from(context, work_path, source)
        }),
        None => analyzer::prepare_source_snapshot(context, work_path),
    }
}

/// Хешер git'ом файлов, которые выгрузка записала в набор, — для памяти хешами git. Что ему
/// нужно: место набора в рабочей копии git и записи индекса под ним, или ничего вне git.
/// Выгрузка индекс не трогает, поэтому его читают, пока она идёт (#574); правку индекса за эти
/// секунды хешер не увидит — как не видел и правку после чтения.
pub(crate) struct DumpHashing {
    placed: Option<(PathBuf, String, Vec<GitIndexEntry>)>,
}

impl DumpHashing {
    pub(crate) fn of(set: &SourceSetContext) -> Self {
        let placed = toplevel_of(set.path()).and_then(|toplevel| {
            let root = relative_root(set.path(), &toplevel)?;
            let index =
                git::index_entries_under(&toplevel, std::slice::from_ref(&root), &mut || false)
                    .ok()?;
            Some((toplevel, root, index))
        });
        tracing::debug!(
            source_set = %set.name(),
            entries = placed.as_ref().map_or(0, |(_, _, index)| index.len()),
            "the git index was read for the dump hasher"
        );
        Self { placed }
    }

    /// `f` получает корень набора от корня рабочей копии и хешер, или `None` вне git.
    pub(crate) fn with<T>(&self, f: impl FnOnce(Option<(&str, &mut BlobHasher<'_>)>) -> T) -> T {
        match &self.placed {
            Some((toplevel, root, index)) => {
                let mut hasher = blob_hasher(toplevel, index);
                f(Some((root, &mut hasher)))
            }
            None => f(None),
        }
    }
}

/// Корень рабочей копии git, в которой лежит `path`, если набор не в подмодуле и в репозитории
/// хешей SHA-1.
fn toplevel_of(path: &Path) -> Option<PathBuf> {
    match git::placement_of(path, &mut || false) {
        Ok(Placement::Inside { toplevel, .. }) => Some(toplevel),
        _ => None,
    }
}

/// Хешер, который отвечает тем, что сохранил бы `git add`: записи индекса этих путей идут во
/// временный индекс, и преобразование концов строк видит то, что лежит в индексе.
fn blob_hasher<'v>(
    toplevel: &'v Path,
    index: &'v [GitIndexEntry],
) -> impl FnMut(&[String], &mut dyn FnMut() -> bool) -> Result<Vec<String>, BlobHashError> + 'v {
    let mut by_path: HashMap<&str, Vec<&GitIndexEntry>> = HashMap::new();
    for entry in index {
        by_path.entry(entry.path.as_str()).or_default().push(entry);
    }
    move |paths, interrupted| {
        let known: Vec<&GitIndexEntry> = paths
            .iter()
            .filter_map(|path| by_path.get(path.as_str()))
            .flatten()
            .copied()
            .collect();
        git::blob_ids_as_added(toplevel, paths, &known, interrupted).map_err(|error| match error {
            GitQueryError::Interrupted => BlobHashError::Interrupted,
            other => BlobHashError::Failed(other.to_string()),
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
        let runner_dirs = analyzer::runner_dirs_below(context, work_path);
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
        repos[repo]
            .excluded
            .extend(runner_dirs.iter().map(|dir| join(&root, dir)));
        choices.push(Choice::Git {
            repo,
            root,
            runner_dirs,
        });
    }

    let mut views = Vec::with_capacity(repos.len());
    let mut unusable = vec![None; repos.len()];
    for (index, repo) in repos.into_iter().enumerate() {
        let answer = match ask {
            Ask::IgnoreRules => {
                git::ignore_view(&repo.toplevel, &repo.info_exclude, &repo.roots, interrupted)
            }
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
            // Хеши SHA-256 раннер не считает: такой репозиторий читается файл за файлом.
            Ok(view) if view.sha256 => {
                unusable[index] = Some(ScanReason::Sha256Repository);
                views.push(view);
            }
            Ok(view) => views.push(view),
            Err(GitQueryError::Interrupted) => return None,
            Err(error) => {
                tracing::warn!(toplevel = %repo.toplevel.display(), %error, "git failed; the sets' files are read");
                unusable[index] = Some(ScanReason::GitFailed);
                views.push(GitWorktree::default());
            }
        }
    }
    for choice in &mut choices {
        if let Choice::Git { repo, .. } = choice {
            if let Some(reason) = unusable[*repo] {
                *choice = Choice::Scan(reason);
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
        // Память хешей байтов: её сравнивает чтение файлов.
        crate::change_detection::analyzer::rescan_and_commit_full(&project.set(), &project.work())
            .expect("memory of the bytes");
        project.write("Module.bsl", "two");
        let index = project.dir.path().join(".git").join("index");
        let kept = fs::read(&index).expect("index");
        fs::write(&index, "broken").expect("break");

        let analysis = project.analyze();
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::GitFailed))
        );
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([("Module.bsl".to_owned(), ChangeKind::Modified)])
        );

        // Память хешами git без git ни с чем не сравнить: набор грузится целиком.
        fs::write(&index, kept).expect("repair");
        project.remember();
        fs::write(&index, "broken").expect("break");
        let analysis = project.analyze();
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::GitFailed))
        );
        assert!(matches!(analysis.outcome, Ok(AnalysisOutcome::Fallback)));
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
        crate::change_detection::analyzer::rescan_and_commit_full(&project.set(), &project.work())
            .expect("memory of the bytes");
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

        assert!(refuse_hidden_sets(&[project.set()], &project.work()).is_err());
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
        let Err(AppError::Refused(refusal)) = refuse_hidden_sets(&[project.set()], &project.work())
        else {
            panic!("expected a refusal");
        };
        assert!(
            refusal.message().contains("Module.bsl"),
            "{}",
            refusal.message()
        );
    }

    /// Файл, закоммиченный с CRLF до `core.autocrlf=true`: `git add` его не нормализует, и
    /// хеш памяти — тот, что в индексе, а не тот, что дал бы `git hash-object` (#575).
    fn crlf_committed_project() -> Project {
        let project = Project::new(&[("Module.bsl", "a\r\nb\r\n")]);
        run_git(project.dir.path(), &["config", "core.autocrlf", "true"]);
        project
    }

    /// Полный pull под git: память — хеши опубликованного дерева, какими их сохранил бы
    /// `git add`, индекс git обновлён в этой же команде, а правка после публикации остаётся
    /// изменением.
    #[test]
    fn a_full_pull_remembers_the_published_tree_as_git_add_would_store_it() {
        let project = crlf_committed_project();
        project.write("Other.bsl", "x\r\n");
        let staging = project.work().join("stage");
        fs::create_dir_all(&staging).expect("stage");
        fs::write(staging.join("Module.bsl"), "a\r\nb\r\n").expect("dumped");
        fs::write(staging.join("Other.bsl"), "x\r\n").expect("dumped");
        let prepared = prepare_full_snapshot(&project.set(), &staging).expect("staged");
        // Публикация переносит файлы, не переписывая их.
        for name in ["Module.bsl", "Other.bsl"] {
            fs::remove_file(project.root().join(name)).expect("old");
            fs::rename(staging.join(name), project.root().join(name)).expect("publish");
        }
        let memory =
            super::full_pull_memory(&project.set(), &project.work(), prepared).expect("memory");
        commit_full_snapshot(&project.set(), &project.work(), &memory).expect("commit");

        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// Правка, сделанная после публикации полной выгрузки, памятью не становится.
    #[test]
    fn an_edit_after_a_full_pull_was_published_stays_a_change() {
        let project = crlf_committed_project();
        let staging = project.work().join("stage");
        fs::create_dir_all(&staging).expect("stage");
        fs::write(staging.join("Module.bsl"), "a\r\nb\r\n").expect("dumped");
        let prepared = prepare_full_snapshot(&project.set(), &staging).expect("staged");
        fs::remove_file(project.root().join("Module.bsl")).expect("old");
        fs::rename(
            staging.join("Module.bsl"),
            project.root().join("Module.bsl"),
        )
        .expect("publish");
        std::thread::sleep(std::time::Duration::from_millis(50));
        project.write("Module.bsl", "a\r\nb\r\nedited\r\n");
        let memory =
            super::full_pull_memory(&project.set(), &project.work(), prepared).expect("memory");
        commit_full_snapshot(&project.set(), &project.work(), &memory).expect("commit");

        assert_eq!(
            changes(&project.analyze()),
            BTreeMap::from([("Module.bsl".to_owned(), ChangeKind::Added)])
        );
    }

    /// Память после `push` под git: правленый файл, закоммиченный с CRLF, запоминается тем
    /// хешем, что сохранил бы `git add`, и сразу после этого изменений нет.
    #[test]
    fn the_memory_after_a_push_holds_what_git_add_would_store() {
        let project = crlf_committed_project();
        project.write("Module.bsl", "a\r\nb\r\nc\r\n");
        project.remember();
        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// Память одного хешера с другим не сравнивается (#575): память хешей байтов — например,
    /// записанная до того, как каталог стал рабочей копией git, — под git сравнивается своим
    /// хешером, чтением файлов, без ложной полной загрузки; полная отправка пишет её хешами git.
    #[test]
    fn a_memory_of_the_bytes_is_compared_by_its_own_hasher_under_git() {
        let project = Project::new(&[("Module.bsl", "one")]);
        crate::change_detection::analyzer::rescan_and_commit_full(&project.set(), &project.work())
            .expect("memory of the bytes");
        let analysis = project.analyze();
        assert_eq!(
            analysis.detection,
            Some(ChangeDetection::scan(ScanReason::BytesMemory))
        );
        assert_eq!(changes(&analysis), BTreeMap::new());

        project.remember();
        let analysis = project.analyze();
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// Неотслеживаемый `.gitignore` git применяет так же, как отслеживаемый.
    #[test]
    fn an_untracked_gitignore_inside_the_set_counts() {
        let project = Project::new(&[("Module.bsl", "m")]);
        project.remember();
        project.write("Sub/.gitignore", "*.bsl\n");
        project.write("Sub/Inner.bsl", "i");
        run_git(project.dir.path(), &["add", "-f", "src/cf/Sub/Inner.bsl"]);
        run_git(project.dir.path(), &["commit", "-qm", "inner"]);
        project.remember();
        let Err(AppError::Refused(refusal)) = project.analyze_with(false) else {
            panic!("expected a refusal");
        };
        assert!(
            refusal.message().contains("Sub/Inner.bsl"),
            "{}",
            refusal.message()
        );
    }

    /// Сторож повторного появления (#575): git и обход выбирают одни и те же файлы набора и дают
    /// им одни и те же хеши — на дереве, которое задевает каждый предикат обхода: служебные и
    /// порождённые каталоги на любой глубине, опись версий и её временный файл, зонд выгрузки,
    /// `workPath` и `localPath` внутри корня, `.local` по имени и файл вне индекса.
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
            (".local/db/Stand.xml", "<s/>"),
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
        project.write("personal/db/Stand.xml", "<s/>");
        let set = project
            .set()
            .with_local_path(&project.root().join("personal"));
        let work = project.root().join(".work-inside");

        rescan_and_commit_full(&set, &work).expect("by git");
        let by_git = remembered(&set, &work);
        crate::change_detection::analyzer::rescan_and_commit_full(&set, &work).expect("by scan");
        let by_scan = remembered(&set, &work);
        // Без фильтров хеши git и хеши байтов совпадают; память же помнит своего хешера.
        assert!(matches!(
            analyze_context(&set, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        rescan_and_commit_full(&set, &work).expect("by git again");
        assert_eq!(by_git, by_scan);
        assert!(by_git.contains_key("Untracked.bsl"));
        assert!(by_git.contains_key("Catalogs/Items/Ext/ObjectModule.bsl"));
        assert!(!by_git.keys().any(|path| {
            path.starts_with("Catalogs/build/")
                || path.starts_with("personal/")
                || path.starts_with(".local/")
        }));

        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, std::slice::from_ref(&set), &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }

    /// Выгрузка по изменившемуся под git запоминает записанное хешами git: при `core.autocrlf`
    /// файл, который она переписала с CRLF, сразу после неё не изменение
    /// (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`).
    #[test]
    fn a_pull_by_changes_under_git_remembers_what_it_wrote_by_git_ids() {
        use crate::change_detection::analyzer::{
            list_after_dump, mark_dump_start, observe_dump_writes, watch_before_dump,
        };

        // Закоммичен с CRLF до `core.autocrlf=true`: `git add` его не нормализует.
        let project = crlf_committed_project();
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
        super::DumpHashing::of(&set)
            .with(|blobs| {
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

    /// `localPath` внутри набора по пути git — не исходник (#596): его `.gitignore` со строкой
    /// `*`, даже закоммиченный, не прячет файлов набора и отказа `source_set_ignored_by_git` не
    /// даёт — и когда память, записанная без `localPath`, помнит его файлы, — а новый файл в нём
    /// не изменение.
    #[test]
    fn a_local_path_inside_the_set_is_not_a_source_under_git() {
        let project = Project::new(&[("Module.bsl", "m")]);
        project.write("personal/.gitignore", "*\n");
        project.write("personal/db/Stand.xml", "<s/>");
        // Закоммиченный `.gitignore` каталога git читает: правило `*` в силе.
        run_git(
            project.dir.path(),
            &["add", "-f", "src/cf/personal/.gitignore"],
        );
        run_git(project.dir.path(), &["commit", "-qm", "local"]);
        // Память, которая помнит файл `localPath`: так его помнил обход до #596.
        crate::change_detection::analyzer::rescan_and_commit_full(&project.set(), &project.work())
            .expect("memory");
        assert!(remembered(&project.set(), &project.work()).contains_key("personal/db/Stand.xml"));

        let set = project
            .set()
            .with_local_path(&project.root().join("personal"));
        let work = project.work();
        let execution = ExecutionContext::cli(CommandName::Status);
        let analysis = analyze(&execution, std::slice::from_ref(&set), &work, false, true)
            .expect("the local directory hides no file of the set")
            .remove(0);
        // Память хешей байтов сравнивается чтением файлов: файл `localPath` из неё ушёл.
        assert_eq!(
            changes(&analysis),
            BTreeMap::from([
                (".gitignore".to_owned(), ChangeKind::Deleted),
                ("Stand.xml".to_owned(), ChangeKind::Deleted),
            ])
        );
        refuse_hidden_sets(std::slice::from_ref(&set), &work)
            .expect("a full pull is not refused either");

        rescan_and_commit_full(&set, &work).expect("memory");
        project.write("personal/db/Other.xml", "<o/>");
        let analysis = analyze(&execution, std::slice::from_ref(&set), &work, false, true)
            .expect("analysis")
            .remove(0);
        assert_eq!(analysis.detection, Some(ChangeDetection::git(false)));
        assert_eq!(changes(&analysis), BTreeMap::new());
    }
}
