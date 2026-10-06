use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use walkdir::WalkDir;

/// Покрывает ли имя `relative` в каталоге `dir` шаблон из `.gitignore` рабочей
/// копии.
///
/// Считается только шаблон, который уезжает вместе с репозиторием: файл
/// `.gitignore` внутри рабочей копии. `.git/info/exclude` и `core.excludesFile`
/// живут на одной машине — у коллеги их нет, и опись, «покрытая» ими здесь, у него
/// уйдёт в коммит. Решает последний совпавший шаблон: отрицание (`!имя`) значит
/// «не покрыто».
///
/// Проба идёт с `--no-index`: без него гит объявляет отслеживаемый файл
/// неигнорируемым даже под шаблоном, и генератор `.gitignore` дописывал бы шаблон
/// при каждом запуске. Вопрос здесь — о шаблоне, а не об индексе; об индексе
/// спрашивает [`tracking_of`].
///
/// Путь передаётся от `dir`, а не абсолютным: абсолютный путь гит сравнивает с
/// корнем рабочей копии буквально, и ссылка в пути превращала бы ответ в «вне
/// репозитория».
///
/// `None` — гита нет, `dir` вне рабочей копии или гит вернул ошибку: тогда решает
/// текст самого `.gitignore`.
pub fn ignored_by_worktree_gitignore(dir: &Path, relative: &Path) -> Option<bool> {
    use std::io::Write;

    let unknown = |reason: &dyn std::fmt::Display| -> Option<bool> {
        tracing::debug!(
            dir = %dir.display(),
            path = %relative.display(),
            %reason,
            "git ignore coverage is unknown"
        );
        None
    };

    // `-z` гит принимает только вместе с `--stdin`: путь уходит на вход, ответ
    // приходит полями через NUL, и никакое имя не ломает разбор.
    let mut child = match Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "-z", "--stdin", "--verbose", "--no-index"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return unknown(&format_args!("git check-ignore failed to run: {error}")),
    };
    let mut request = relative.as_os_str().as_encoded_bytes().to_vec();
    request.push(0);
    // Вход закрывается до ожидания: иначе гит ждал бы конца ввода вечно.
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(&request));
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return unknown(&format_args!("git check-ignore failed: {error}")),
    };
    match written {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            return unknown(&format_args!(
                "failed to write to git check-ignore: {error}"
            ))
        }
        None => return unknown(&"git check-ignore has no standard input"),
    }

    match output.status.code() {
        Some(0) => Some(match_comes_from_worktree_gitignore(&output.stdout)),
        Some(1) => Some(false),
        Some(code) => unknown(&format_args!("git check-ignore exited with {code}")),
        None => unknown(&"git check-ignore was terminated by a signal"),
    }
}

/// Разбирает ответ `git check-ignore -z --verbose`: источник, строка, шаблон, путь.
///
/// Файл из `core.excludesFile` гит называет абсолютным путём, а файлы игнора
/// внутри дерева — путём от корня рабочей копии. «Внутри дерева» поэтому — это
/// относительный путь к файлу с именем `.gitignore` вне каталога `.git`.
fn match_comes_from_worktree_gitignore(stdout: &[u8]) -> bool {
    let mut fields = stdout.split(|byte| *byte == 0);
    let (Some(source), Some(_line), Some(pattern)) = (fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    if source.is_empty() || pattern.starts_with(b"!") {
        return false;
    }
    let source = path_from_bytes(source);
    source.is_relative()
        && source.file_name() == Some(std::ffi::OsStr::new(".gitignore"))
        && !source
            .components()
            .any(|component| component.as_os_str() == ".git")
}

/// Лежит ли файл в индексе гита.
///
/// Состояний три, как и у [`UncommittedWork`]: незнание — самостоятельный ответ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tracking {
    /// Гит отвечает: файла в индексе нет.
    Untracked,
    /// Файл в индексе. Путь дан от корня рабочей копии — в том виде, в каком его
    /// принимает `git rm --cached`, запущенный из корня.
    Tracked(PathBuf),
    /// Ответа нет, причина названа: гита нет, путь вне рабочей копии, гит вернул
    /// ошибку.
    Unknown(String),
}

/// Спрашивает гит, лежит ли `path` в индексе.
///
/// Каталога файла может не быть на диске — например, до первой выгрузки. Гит
/// запускается из ближайшего существующего предка, а путь передаётся от него:
/// абсолютный путь гит сравнивает с корнем рабочей копии буквально, и ссылка в
/// пути превращала бы ответ в «вне репозитория».
pub fn tracking_of(path: &Path) -> Tracking {
    let Some(anchor) = path.ancestors().skip(1).find(|dir| dir.is_dir()) else {
        return Tracking::Unknown(format!("no existing directory above '{}'", path.display()));
    };
    let Ok(relative) = path.strip_prefix(anchor) else {
        return Tracking::Unknown(format!(
            "'{}' is not under '{}'",
            path.display(),
            anchor.display()
        ));
    };

    let output = match Command::new("git")
        // Имя файла — не шаблон: `*` или `[` в пути не должны ничего расширять.
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(anchor)
        .args(["ls-files", "-z", "--full-name", "--cached", "--"])
        .arg(relative)
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) => return Tracking::Unknown(format!("git ls-files failed to run: {error}")),
    };

    if !output.status.success() {
        return Tracking::Unknown(match output.status.code() {
            Some(code) => format!("git ls-files exited with {code}"),
            None => "git ls-files was terminated by a signal".to_owned(),
        });
    }

    match output
        .stdout
        .split(|byte| *byte == 0)
        .find(|entry| !entry.is_empty())
    {
        Some(entry) => Tracking::Tracked(path_from_bytes(entry)),
        None => Tracking::Untracked,
    }
}

/// Что в каталоге пропадёт безвозвратно, если его содержимое заменить.
///
/// Состояний три, а не два: незнание — самостоятельный ответ, и смешивать его с
/// «терять нечего» нельзя. Что с ним делать, решает вызывающий: сторож замены
/// приравнивает его к безвозвратному и считает потерей каждый файл каталога.
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
    Unknown(NoAnswer),
}

/// Почему гит не ответил. Различается затем, чтобы совет был верным: каталог вне
/// рабочей копии берут под контроль версий, а упавший гит в рабочей копии чинят.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoAnswer {
    /// Каталог не лежит ни в какой рабочей копии.
    OutsideRepository(String),
    /// Гита нет, он не запустился или упал внутри рабочей копии (`safe.directory`,
    /// сломанный индекс), или перечень вышел неполным. Причина несёт первую строку
    /// stderr гита, если он её написал.
    Failed(String),
}

impl NoAnswer {
    pub fn reason(&self) -> &str {
        match self {
            Self::OutsideRepository(reason) | Self::Failed(reason) => reason,
        }
    }
}

/// Лежит ли `dir` в рабочей копии: есть ли `.git` в нём или выше. Решает файловая
/// система, а не текст отказа гита: прозе инструмента решения не доверяются.
fn inside_a_worktree(dir: &Path) -> bool {
    let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    absolute
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists())
}

/// Первый подкаталог `dir`, который не удалось прочесть, — если такой есть.
///
/// Замерено: нечитаемый подкаталог даёт у `git status` нулевой выход и пустой список
/// вместо своего содержимого. Предупреждение об этом приходит прозой в stderr, а прозе
/// решения не доверяются, поэтому полноту ответа проверяет обход самого каталога.
fn unreadable_directory_in(dir: &Path) -> Option<PathBuf> {
    WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .find_map(|entry| match entry {
            Ok(_) => None,
            Err(error) => Some(error.path().unwrap_or(dir).to_path_buf()),
        })
}

/// Первая непустая строка stderr — то, что гит сказал о причине.
fn first_line(stderr: &str) -> Option<&str> {
    stderr.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Спрашивает гит, что в `dir` не восстановить после замены каталога.
///
/// Безвозвратно — это правка, живущая только на диске: незафиксированное изменение,
/// файл вне учёта и файл в игноре. Проиндексированное сюда не относится: его
/// содержимое лежит в `.git/index` и достаётся оттуда.
///
/// `regenerated` — имена файлов в корне `dir`, которые сама замена пишет заново:
/// их прежнее содержимое не потеря, а вопрос о них отказывал бы в каждой замене.
/// Имена сравниваются буквально и только в корне `dir`.
pub fn uncommitted_work_in(dir: &Path, regenerated: &[&str]) -> UncommittedWork {
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
        .args(
            regenerated
                .iter()
                .map(|name| format!(":(exclude,literal){name}")),
        )
        // Вопрос ничего не пишет: без этого `status` обновляет индекс рабочей копии, а
        // превью следа не оставляет.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return UncommittedWork::Unknown(NoAnswer::Failed(format!(
                "git status failed to run: {error}"
            )))
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let exit = match output.status.code() {
            Some(code) => format!("git status exited with {code}"),
            None => "git status was terminated by a signal".to_owned(),
        };
        // Первая строка stderr только называет причину человеку; выбор совета решает
        // файловая система.
        let reason = match first_line(&stderr) {
            Some(line) => format!("{exit}: {line}"),
            None => exit,
        };
        return UncommittedWork::Unknown(if inside_a_worktree(dir) {
            NoAnswer::Failed(reason)
        } else {
            NoAnswer::OutsideRepository(reason)
        });
    }

    // При нулевом выходе ответ получен; предупреждения в stderr (например, о замене концов
    // строк) его не отменяют и уходят только в журнал.
    if let Some(warning) = first_line(&stderr) {
        tracing::debug!(dir = %dir.display(), warning, "git status warning ignored");
    }
    // Полнота перечня — условие, без которого он ничего не значит.
    if let Some(unreadable) = unreadable_directory_in(dir) {
        return UncommittedWork::Unknown(NoAnswer::Failed(format!(
            "'{}' could not be read, so git status could not list it",
            unreadable.display()
        )));
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
    use crate::platform::test_git::{init_git_repo, run_git};
    use std::fs;
    use tempfile::{tempdir, TempDir};

    /// Репозиторий с одним зафиксированным файлом в `src/cf`.
    fn repo_with_committed_source() -> TempDir {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
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

    fn source_dir(repo: &TempDir) -> PathBuf {
        repo.path().join("src").join("cf")
    }

    #[test]
    fn a_clean_tree_has_nothing_to_lose() {
        let repo = repo_with_committed_source();
        assert_eq!(
            uncommitted_work_in(&source_dir(&repo), &[]),
            UncommittedWork::Nothing
        );
    }

    #[test]
    fn an_absent_directory_has_nothing_to_lose() {
        let repo = repo_with_committed_source();
        assert_eq!(
            uncommitted_work_in(&repo.path().join("src").join("never-was"), &[]),
            UncommittedWork::Nothing
        );
    }

    #[test]
    fn an_untracked_file_is_at_risk() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("hand-written.xml"), "mine\n").expect("write");
        assert_eq!(
            uncommitted_work_in(&source_dir(&repo), &[]),
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
            uncommitted_work_in(&source_dir(&repo), &[]),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/scratch.local.xml")])
        );
    }

    /// Файл, который замена пишет заново, не потеря — но только в корне каталога:
    /// одноимённый файл глубже остаётся под защитой.
    #[test]
    fn a_regenerated_file_at_the_root_is_not_at_risk() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
        run_git(repo.path(), &["add", ".gitignore"]);
        run_git(repo.path(), &["commit", "-qm", "ignore"]);
        let source = source_dir(&repo);
        fs::write(source.join("ConfigDumpInfo.xml"), "<info/>\n").expect("root inventory");
        fs::create_dir_all(source.join("nested")).expect("nested dir");
        fs::write(source.join("nested").join("ConfigDumpInfo.xml"), "mine\n").expect("nested");

        assert_eq!(
            uncommitted_work_in(&source, &["ConfigDumpInfo.xml"]),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/nested/ConfigDumpInfo.xml")])
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
            uncommitted_work_in(&source_dir(&repo), &[]),
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
            uncommitted_work_in(&source_dir(&repo), &[]),
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
            uncommitted_work_in(&source_dir(&repo), &[]),
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
        init_git_repo(root);
        fs::write(root.join("aM.xml"), "renamed away\n").expect("write");
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-qm", "init"]);
        run_git(root, &["mv", "aM.xml", "renamed.xml"]);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert_eq!(
            uncommitted_work_in(root, &[]),
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
            uncommitted_work_in(&source_dir(&repo), &[]),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/precious.xml")])
        );
    }

    /// Переименование бывает и в рабочей колонке — гит замечает перемещение,
    /// сделанное мимо него. Прежнее имя там тоже идёт отдельным полем.
    #[test]
    fn a_rename_seen_in_the_worktree_column_does_not_invent_a_loss() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
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
            uncommitted_work_in(root, &[]),
            UncommittedWork::AtRisk(vec![PathBuf::from("hand-written.xml")])
        );
    }

    #[test]
    fn a_committed_file_is_tracked_under_its_repository_path() {
        let repo = repo_with_committed_source();
        assert_eq!(
            tracking_of(&source_dir(&repo).join("Configuration.xml")),
            Tracking::Tracked(PathBuf::from("src/cf/Configuration.xml"))
        );
    }

    #[test]
    fn a_file_outside_the_index_is_untracked() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("hand-written.xml"), "mine\n").expect("write");
        assert_eq!(
            tracking_of(&source_dir(&repo).join("hand-written.xml")),
            Tracking::Untracked
        );
    }

    /// Каталога ещё нет — гит спрашивают из ближайшего существующего предка.
    #[test]
    fn a_file_in_an_absent_directory_is_untracked() {
        let repo = repo_with_committed_source();
        assert_eq!(
            tracking_of(&repo.path().join("never").join("was").join("file.xml")),
            Tracking::Untracked
        );
    }

    /// Имя файла — не шаблон: `*` не должен найти соседа.
    #[test]
    fn a_name_is_not_a_pattern() {
        let repo = repo_with_committed_source();
        assert_eq!(
            tracking_of(&source_dir(&repo).join("*.xml")),
            Tracking::Untracked
        );
    }

    #[test]
    fn a_file_outside_a_worktree_has_unknown_tracking() {
        let dir = tempdir().expect("tempdir");
        let answer = tracking_of(&dir.path().join("file.xml"));
        assert!(
            matches!(answer, Tracking::Unknown(_)),
            "expected Unknown, got {answer:?}"
        );
    }

    /// Над отслеживаемым файлом гит без `--no-index` шаблон не признаёт: проба
    /// обязана видеть шаблон, иначе генератор дописывает его снова и снова.
    #[test]
    fn a_pattern_covers_a_tracked_file() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join(".gitignore"), "Configuration.xml\n").expect("gitignore");
        assert_eq!(
            ignored_by_worktree_gitignore(&source_dir(&repo), Path::new("Configuration.xml")),
            Some(true)
        );
    }

    /// `.git/info/exclude` в репозиторий не попадает: у коллеги этого шаблона нет.
    #[test]
    fn a_pattern_in_info_exclude_does_not_count() {
        let repo = repo_with_committed_source();
        fs::write(
            repo.path().join(".git").join("info").join("exclude"),
            "ConfigDumpInfo.xml\n",
        )
        .expect("exclude");
        assert_eq!(
            ignored_by_worktree_gitignore(&source_dir(&repo), Path::new("ConfigDumpInfo.xml")),
            Some(false)
        );
    }

    /// `core.excludesFile` — настройка одной машины, даже если файл зовут `.gitignore`.
    #[test]
    fn a_pattern_in_the_excludes_file_does_not_count() {
        let repo = repo_with_committed_source();
        let elsewhere = tempdir().expect("tempdir");
        let excludes = elsewhere.path().join(".gitignore");
        fs::write(&excludes, "ConfigDumpInfo.xml\n").expect("excludes");
        let excludes = excludes.to_str().expect("utf-8 temp path");
        run_git(repo.path(), &["config", "core.excludesFile", excludes]);
        assert_eq!(
            ignored_by_worktree_gitignore(&source_dir(&repo), Path::new("ConfigDumpInfo.xml")),
            Some(false)
        );
    }

    /// Шаблон во вложенном `.gitignore` уезжает с репозиторием так же, как корневой.
    #[test]
    fn a_pattern_in_a_nested_gitignore_counts() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join(".gitignore"), "ConfigDumpInfo.xml\n").expect("nested");
        assert_eq!(
            ignored_by_worktree_gitignore(&source_dir(&repo), Path::new("ConfigDumpInfo.xml")),
            Some(true)
        );
    }

    /// Отрицание — последнее слово: имя не покрыто.
    #[test]
    fn a_negated_pattern_does_not_cover() {
        let repo = repo_with_committed_source();
        fs::write(
            repo.path().join(".gitignore"),
            "ConfigDumpInfo.xml\n!ConfigDumpInfo.xml\n",
        )
        .expect("gitignore");
        assert_eq!(
            ignored_by_worktree_gitignore(&source_dir(&repo), Path::new("ConfigDumpInfo.xml")),
            Some(false)
        );
    }

    #[test]
    fn ignore_coverage_outside_a_worktree_is_unknown() {
        let dir = tempdir().expect("tempdir");
        assert_eq!(
            ignored_by_worktree_gitignore(dir.path(), Path::new("ConfigDumpInfo.xml")),
            None
        );
    }

    #[test]
    fn a_directory_outside_a_worktree_is_unknown() {
        let dir = tempdir().expect("tempdir");
        let answer = uncommitted_work_in(dir.path(), &[]);
        assert!(
            matches!(
                answer,
                UncommittedWork::Unknown(NoAnswer::OutsideRepository(_))
            ),
            "expected Unknown outside a repository, got {answer:?}"
        );
    }

    /// Гит упал внутри рабочей копии: это не «вне репозитория», и причина несёт то,
    /// что гит сказал.
    #[test]
    fn a_failing_git_inside_a_worktree_names_its_reason() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join(".git").join("index"), "x").expect("break the index");

        let answer = uncommitted_work_in(&source_dir(&repo), &[]);

        let UncommittedWork::Unknown(NoAnswer::Failed(reason)) = answer else {
            panic!("expected a failed git, got {answer:?}");
        };
        assert!(reason.contains("exited with"), "{reason}");
        assert!(reason.contains("index"), "{reason}");
    }

    /// Предупреждение в stderr при нулевом выходе (у гита на Windows — о замене концов
    /// строк) ответа не отменяет: найденное безвозвратное остаётся найденным.
    #[cfg(unix)]
    #[test]
    fn a_warning_on_a_successful_status_keeps_the_answer() {
        use std::os::unix::fs::PermissionsExt;

        let repo = repo_with_committed_source();
        let hook = repo.path().join("warn.sh");
        fs::write(
            &hook,
            "#!/bin/sh\necho 'warning: in the working copy of x, LF will be replaced by CRLF' >&2\nexit 1\n",
        )
        .expect("hook");
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod");
        run_git(
            repo.path(),
            &[
                "config",
                "core.fsmonitor",
                hook.to_str().expect("utf-8 path"),
            ],
        );
        fs::write(source_dir(&repo).join("hand-written.xml"), "mine\n").expect("write");

        assert_eq!(
            uncommitted_work_in(&source_dir(&repo), &[]),
            UncommittedWork::AtRisk(vec![PathBuf::from("src/cf/hand-written.xml")])
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

        let answer = uncommitted_work_in(&source_dir(&repo), &[]);

        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o755)).expect("restore");
        assert!(
            matches!(answer, UncommittedWork::Unknown(_)),
            "expected Unknown, got {answer:?}"
        );
    }
}
