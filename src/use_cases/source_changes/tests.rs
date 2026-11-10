use super::{analyze, commit_full_pull, refuse_hidden_sets, rescan_and_commit_full};
use crate::change_detection::analyzer::{
    analyze_context, prepare_full_snapshot, AnalysisOutcome, ChangeKind, ContextAnalysis,
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

/// Полный pull под git запоминает опубликованное дерево хешами git: при `core.autocrlf` сразу
/// после него изменений нет, хотя байты на диске с CRLF.
#[test]
fn a_full_pull_remembers_the_published_tree_by_git_ids() {
    let project = Project::new(&[("Module.bsl", "one\ntwo\n")]);
    run_git(project.dir.path(), &["config", "core.autocrlf", "true"]);
    project.write("Module.bsl", "one\r\ntwo\r\n");
    let prepared = prepare_full_snapshot(&project.set(), &project.root()).expect("staged");
    commit_full_pull(&project.set(), &project.work(), &prepared).expect("memory");

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
    ]);
    project.write("Untracked.bsl", "new");
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
