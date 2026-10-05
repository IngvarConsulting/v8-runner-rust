//! Тестовый помощник: временный репозиторий гита без чужих настроек.

use std::path::Path;
use std::process::{Command, Stdio};

/// Запускает `git -C dir <args>` и требует успеха.
pub(crate) fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Заводит в `dir` пустой репозиторий с веткой `main` и автором для коммитов.
pub(crate) fn init_git_repo(dir: &Path) {
    run_git(dir, &["init", "-q", "-b", "main", "."]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}
