use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Returns Git's effective ignore decision for `path`.
///
/// `None` means Git is unavailable, `path` is outside a worktree, or Git reported
/// an execution error that should fall back to local `.gitignore` editing.
pub fn check_ignored(path: &Path) -> Option<bool> {
    let workdir = path.parent().unwrap_or_else(|| Path::new("."));
    let status = Command::new("git")
        .arg("-C")
        .arg(workdir)
        .args(["check-ignore", "--quiet", "--"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;

    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// Что в каталоге пропадёт безвозвратно, если его содержимое заменить.
///
/// Состояний три, а не два: незнание — самостоятельный ответ, и приравнивать его
/// к худшему нельзя. Каталог вне рабочей копии — законное `Unknown`, а не отказ:
/// раннер работает и без гита вовсе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncommittedWork {
    /// Гит отвечает: всё, что здесь лежит, он вернёт.
    Nothing,
    /// Перечисленного нет нигде, кроме самого каталога.
    ///
    /// Пути даны от корня рабочей копии, а не от опрошенного каталога: так их
    /// нельзя спутать между наборами исходников.
    AtRisk(Vec<PathBuf>),
    /// Ответа нет, причина названа.
    Unknown(String),
}

/// Спрашивает гит, что в `dir` не восстановить после замены каталога.
///
/// Безвозвратно — это правка, живущая только на диске: незафиксированное изменение,
/// файл вне учёта и файл в игноре. Проиндексированное сюда не относится: его
/// содержимое лежит в `.git/index` и достаётся оттуда.
pub fn uncommitted_work_in(dir: &Path) -> UncommittedWork {
    if !dir.exists() {
        return UncommittedWork::Nothing;
    }

    let output = match Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "status",
            "--porcelain",
            "-z",
            "--ignored=matching",
            "--untracked-files=all",
            // Без ограничения путём гит докладывает всю рабочую копию: правка в
            // чужом каталоге репозитория выглядела бы угрозой этому.
            "--",
            ".",
        ])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return UncommittedWork::Unknown(format!("git status failed to run: {error}"))
        }
    };

    if !output.status.success() {
        return UncommittedWork::Unknown(match output.status.code() {
            Some(code) => format!("git status exited with {code}"),
            None => "git status was terminated by a signal".to_owned(),
        });
    }

    // Предупреждение приходит в stderr при нулевом выходе, а список при этом
    // молча неполон: нечитаемый подкаталог даёт пустой ответ вместо своего
    // содержимого. Полнота списка — условие, без которого он ничего не значит.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let warning = stderr.lines().find(|line| !line.trim().is_empty());
    if let Some(warning) = warning {
        return UncommittedWork::Unknown(format!("git status reported: {warning}"));
    }

    UncommittedWork::AtRisk(parse_at_risk(&output.stdout)).normalized()
}

impl UncommittedWork {
    fn normalized(self) -> Self {
        match self {
            Self::AtRisk(paths) if paths.is_empty() => Self::Nothing,
            other => other,
        }
    }
}

/// Разбирает `git status --porcelain -z` и оставляет только безвозвратное.
fn parse_at_risk(stdout: &[u8]) -> Vec<PathBuf> {
    let mut at_risk = Vec::new();
    let mut fields = stdout.split(|byte| *byte == 0);

    while let Some(record) = fields.next() {
        if record.len() < 3 {
            continue;
        }
        let index = record[0];
        let worktree = record[1];
        let path = path_from_bytes(&record[3..]);

        // У переименования и копирования следом идёт отдельным полем прежнее имя.
        // Колонка бывает любой: `R ` даёт `git mv`, ` R` — перемещение в рабочем
        // каталоге, замеченное гитом.
        if matches!(index, b'R' | b'C') || matches!(worktree, b'R' | b'C') {
            let _ = fields.next();
        }

        if is_unrecoverable(index, worktree) {
            at_risk.push(path);
        }
    }

    at_risk
}

/// Имя файла ровно тем, чем его отдал гит.
///
/// `-z` затем и нужен, чтобы байты дошли неискажёнными: имя, не являющееся UTF-8,
/// после «мягкого» преобразования перестало бы существовать на диске, а сторож
/// называет его человеку, чтобы тот его нашёл.
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// Восстановимо ли содержимое где-нибудь, кроме рабочего каталога.
fn is_unrecoverable(index: u8, worktree: u8) -> bool {
    match (index, worktree) {
        // Вне учёта и в игноре: содержимого нет больше нигде. Игнор здесь —
        // самый опасный случай, а выглядит он спокойнее прочих.
        (b'?', b'?') | (b'!', b'!') => true,
        // Незавершённое слияние: разметка конфликта живёт только на диске.
        (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => true,
        // `git add -N`: индекс держит пустой blob, содержимое есть только на диске.
        // В рабочей колонке `A` больше ничего не означает.
        (_, b'A') => true,
        // Рабочий каталог разошёлся с индексом — разница есть только здесь.
        // Проиндексированное без правки поверх (` M` против `M `) достаётся из
        // индекса и потерей не считается.
        (_, b'M') | (_, b'T') => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::{tempdir, TempDir};

    /// Репозиторий с одним зафиксированным файлом в `src/cf`.
    fn repo_with_committed_source() -> TempDir {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        run_git(root, &["init", "-q", "-b", "main", "."]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test"]);
        fs::create_dir_all(root.join("src").join("cf")).expect("source dir");
        fs::write(
            root.join("src").join("cf").join("Configuration.xml"),
            "conf\n",
        )
        .expect("configuration");
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-qm", "init"]);
        dir
    }

    fn run_git(dir: &Path, args: &[&str]) {
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

    fn source_dir(repo: &TempDir) -> PathBuf {
        repo.path().join("src").join("cf")
    }

    #[test]
    fn a_clean_tree_has_nothing_to_lose() {
        let repo = repo_with_committed_source();
        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::Nothing
        );
    }

    #[test]
    fn an_absent_directory_has_nothing_to_lose() {
        let repo = repo_with_committed_source();
        assert_eq!(
            uncommitted_work_in(&repo.path().join("src").join("never-was")),
            UncommittedWork::Nothing
        );
    }

    #[test]
    fn an_untracked_file_is_at_risk() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("hand-written.xml"), "mine\n").expect("write");
        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/hand-written.xml")])
        );
    }

    /// Файл в игноре — самый невосстановимый из всех, а без `--ignored` каталог
    /// с ним читается как чистый. Ровно этот случай сторож и обязан поймать.
    #[test]
    fn an_ignored_file_is_at_risk_although_the_tree_looks_clean() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join(".gitignore"), "*.local.xml\n").expect("gitignore");
        run_git(repo.path(), &["add", ".gitignore"]);
        run_git(repo.path(), &["commit", "-qm", "ignore"]);
        fs::write(source_dir(&repo).join("scratch.local.xml"), "mine\n").expect("write");

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/scratch.local.xml")])
        );
    }

    /// Без ограничения путём гит докладывает всю рабочую копию: правка в чужом
    /// каталоге репозитория отказывала бы в выгрузке этого.
    #[test]
    fn a_change_outside_the_asked_directory_is_not_a_threat_to_it() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join("README.md"), "unrelated\n").expect("write");
        fs::create_dir_all(repo.path().join("other")).expect("other dir");
        fs::write(repo.path().join("other").join("notes.md"), "unrelated\n").expect("write");

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::Nothing
        );
    }

    /// Проиндексированное достаётся из `.git/index`: это не потеря.
    #[test]
    fn staged_content_is_recoverable() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("added.xml"), "staged\n").expect("write");
        run_git(repo.path(), &["add", "src/cf/added.xml"]);

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::Nothing
        );
    }

    /// А правка поверх проиндексированного живёт только на диске.
    #[test]
    fn a_worktree_change_on_top_of_a_staged_one_is_at_risk() {
        let repo = repo_with_committed_source();
        let path = source_dir(&repo).join("added.xml");
        fs::write(&path, "staged\n").expect("write");
        run_git(repo.path(), &["add", "src/cf/added.xml"]);
        fs::write(&path, "and then edited\n").expect("write");

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/added.xml")])
        );
    }

    /// У переименования следом идёт отдельным полем прежнее имя. Не считать его —
    /// значит разобрать это имя как очередную запись: `aM.xml` прочитается кодом
    /// состояния `a`/`M` и даст призрачную потерю с путём `xml`.
    ///
    /// Ловится это там, где каталог сам себе репозиторий: внутри подкаталога
    /// прежнее имя начинается с его префикса, и подмена выходит безобидной.
    #[test]
    fn a_rename_does_not_invent_a_loss() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        run_git(root, &["init", "-q", "-b", "main", "."]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test"]);
        fs::write(root.join("aM.xml"), "renamed away\n").expect("write");
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-qm", "init"]);
        run_git(root, &["mv", "aM.xml", "renamed.xml"]);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert_eq!(
            uncommitted_work_in(root),
            UncommittedWork::AtRisk(vec![PathBuf::from("hand-written.xml")])
        );
    }

    /// `git add -N` кладёт в индекс **пустой** blob: содержимое остаётся только на
    /// диске. Колонка индекса при этом пуста, рабочая — `A`, и принять это за
    /// «сохранено» значит стереть файл, которого больше нигде нет.
    #[test]
    fn an_intent_to_add_file_is_at_risk() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("precious.xml"), "only on disk\n").expect("write");
        run_git(repo.path(), &["add", "-N", "src/cf/precious.xml"]);

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo)),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/precious.xml")])
        );
    }

    /// Переименование бывает и в рабочей колонке — гит замечает перемещение,
    /// сделанное мимо него. Прежнее имя там тоже идёт отдельным полем.
    #[test]
    fn a_rename_seen_in_the_worktree_column_does_not_invent_a_loss() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        run_git(root, &["init", "-q", "-b", "main", "."]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test"]);
        fs::write(root.join("aM.xml"), "moved away\n").expect("write");
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-qm", "init"]);
        fs::rename(root.join("aM.xml"), root.join("renamed.xml")).expect("move");
        run_git(root, &["add", "-N", "renamed.xml"]);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        // Перемещение зафиксированного файла потерей не является: содержимое лежит
        // в хранилище объектов. Проверяется здесь другое — что прежнее имя не
        // прочиталось как очередная запись и не породило путь `xml`.
        assert_eq!(
            uncommitted_work_in(root),
            UncommittedWork::AtRisk(vec![PathBuf::from("hand-written.xml")])
        );
    }

    #[test]
    fn a_directory_outside_a_worktree_is_unknown() {
        let dir = tempdir().expect("tempdir");
        let answer = uncommitted_work_in(dir.path());
        assert!(
            matches!(answer, UncommittedWork::Unknown(_)),
            "expected Unknown, got {answer:?}"
        );
    }

    /// Замерено: нечитаемый подкаталог даёт нулевой выход, пустой список и
    /// предупреждение в stderr. Принять это за «чисто» — пропустить уничтожение.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_subdirectory_is_unknown_not_clean() {
        use std::os::unix::fs::PermissionsExt;

        let repo = repo_with_committed_source();
        let hidden = source_dir(&repo).join("sub");
        fs::create_dir_all(&hidden).expect("subdir");
        fs::write(hidden.join("hand-written.xml"), "mine\n").expect("write");
        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o000)).expect("chmod");

        let answer = uncommitted_work_in(&source_dir(&repo));

        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o755)).expect("restore");
        assert!(
            matches!(answer, UncommittedWork::Unknown(_)),
            "expected Unknown, got {answer:?}"
        );
    }
}
