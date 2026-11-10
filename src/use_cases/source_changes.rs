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

/// Записывает память набора после полной выгрузки, уже опубликованной в его каталог. Под git
/// `git status` по корню набора обновляет индекс — переписанные файлы git перечитывает один
/// раз здесь, а не в следующей команде, — и память берёт хеши git опубликованного дерева
/// (#575). Вне git, или если git отказал, пишется `prepared` — дерево, снятое до публикации.
pub(crate) fn commit_full_pull(
    context: &SourceSetContext,
    work_path: &Path,
    prepared: &analyzer::FullSnapshot,
) -> Result<(), ChangeDetectionError> {
    let started = std::time::Instant::now();
    let committed = commit_from_git(context, work_path);
    tracing::info!(
        source_set = %context.name(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        by_git = committed.as_ref().is_ok_and(|by_git| *by_git),
        "memory of a full pull recorded"
    );
    match committed {
        Ok(true) => Ok(()),
        Ok(false) => analyzer::commit_full_snapshot(context, work_path, prepared),
        Err(error) => Err(error),
    }
}

/// Записывает память набора хешами git; `Ok(false)` — набор не в git или git не описал его.
fn commit_from_git(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<bool, ChangeDetectionError> {
    // Публикация уже прошла: память пишется и при отложенной отмене.
    let lookup = lookup(
        std::slice::from_ref(context),
        work_path,
        Ask::Status {
            optional_locks: true,
        },
        &mut || false,
    );
    let Some((
        lookup,
        Choice::Git {
            repo,
            root,
            work_path: work,
        },
    )) = lookup.as_ref().map(|lookup| (lookup, &lookup.choices[0]))
    else {
        return Ok(false);
    };
    let view = &lookup.repos[*repo];
    let mut hasher = blob_hasher(&view.toplevel, true);
    analyzer::commit_full_from_git(
        context,
        work_path,
        GitSource {
            view,
            root: root.clone(),
            work_path: work.clone(),
            hash_blobs: &mut hasher,
        },
    )
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
    let placed = match git::placement_of(set.path(), &mut || false) {
        Ok(Placement::Inside { toplevel, .. }) => {
            relative_root(set.path(), &toplevel).map(|root| (toplevel, root))
        }
        _ => None,
    };
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
        let work = scanner::work_path_inside(context.path(), work_path).and_then(|inside| {
            inside
                .strip_prefix(context.path())
                .ok()
                .map(|relative| slashed(relative))
        });
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
mod tests;
