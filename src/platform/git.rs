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
/// при каждом запуске. Вопрос здесь — о шаблоне, а не об индексе.
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
    let mut child = match git()
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

    let output = match git()
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
    // строк) его не отменяют и уходят только в журнал. Полнота перечня — условие, без
    // которого он ничего не значит: замер #176 — нечитаемый подкаталог даёт нулевой выход
    // и предупреждение в stderr. Непустой stderr только повод проверить полноту; текст не
    // разбирается, решает обход каталога. Без stderr обход не нужен.
    if let Some(warning) = first_line(&stderr) {
        tracing::debug!(dir = %dir.display(), warning, "git status warning ignored");
        if let Some(unreadable) = unreadable_directory_in(dir) {
            return UncommittedWork::Unknown(NoAnswer::Failed(format!(
                "'{}' could not be read, so git status could not list it",
                unreadable.display()
            )));
        }
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

/// Почему вопрос к git о рабочей копии не дал ответа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitQueryError {
    /// Команду отменили; процесс git завершён или, если он мог держать блокировку индекса,
    /// дождался своего конца.
    Interrupted,
    /// Git не запустился: его нет на машине или он не исполняется.
    Unavailable(String),
    /// Git отказал или ответил так, что ответ не разобрать. `code` — код выхода, если git
    /// вышел сам.
    Failed { code: Option<i32>, reason: String },
}

impl GitQueryError {
    fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            code: None,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for GitQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interrupted => f.write_str("git query interrupted"),
            Self::Unavailable(reason) | Self::Failed { reason, .. } => f.write_str(reason),
        }
    }
}

/// Где лежит каталог набора относительно git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Вне рабочей копии, или git не нашёлся.
    Outside(String),
    /// В подмодуле другого репозитория.
    Submodule,
    /// Git внутри рабочей копии отказал.
    Failed(String),
    /// В рабочей копии с корнем `toplevel`.
    Inside {
        toplevel: PathBuf,
        /// `.git/info/exclude` этой рабочей копии.
        info_exclude: PathBuf,
    },
}

/// Что делать с git, если команду отменили, пока он работает.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnCancel {
    /// Завершить: git ничего не держит.
    Kill,
    /// Дождаться конца: git мог взять `.git/index.lock`, а завершённый принудительно он его
    /// не снимает (#575).
    Finish,
}

/// Как часто спрашивается отмена, пока git работает.
const GIT_INTERRUPTION_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// Запускает git и ждёт его, спрашивая отмену. Отменённый git завершается или, при
/// [`OnCancel::Finish`], доходит до конца, и ответ — [`GitQueryError::Interrupted`].
/// Ненулевой выход — [`GitQueryError::Failed`] с кодом выхода и первой строкой stderr;
/// предупреждения при нулевом выходе ответа не отменяют
/// (`INV.USE-CASES.A-GIT-WARNING-DOES-NOT-CANCEL-ITS-ANSWER`).
fn run_git_query(
    mut command: Command,
    stdin: Option<Vec<u8>>,
    on_cancel: OnCancel,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, GitQueryError> {
    use std::io::{Read, Write};

    if interrupted() {
        return Err(GitQueryError::Interrupted);
    }
    let mut child = command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GitQueryError::Unavailable(format!("git failed to run: {error}")))?;
    let input = child.stdin.take();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let read_all = |stream: Option<&mut dyn Read>| -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        if let Some(stream) = stream {
            stream.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    };
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || match (input, stdin) {
            (Some(mut input), Some(bytes)) => input.write_all(&bytes),
            _ => Ok(()),
        });
        let out = scope.spawn(move || read_all(stdout.as_mut().map(|s| s as &mut dyn Read)));
        let err = scope.spawn(move || read_all(stderr.as_mut().map(|s| s as &mut dyn Read)));
        let mut cancelled = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if !cancelled && interrupted() => {
                    cancelled = true;
                    if on_cancel == OnCancel::Kill {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(GitQueryError::Interrupted);
                    }
                }
                Ok(None) => std::thread::sleep(GIT_INTERRUPTION_POLL),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(GitQueryError::failed(format!("git wait failed: {error}")));
                }
            }
        };
        let panicked = || GitQueryError::failed("a thread reading git panicked");
        let written = writer.join().map_err(|_| panicked())?;
        let stdout = out.join().map_err(|_| panicked())?;
        let stderr = err.join().map_err(|_| panicked())?;
        let status = status?;
        if cancelled {
            return Err(GitQueryError::Interrupted);
        }
        let stderr = String::from_utf8_lossy(&stderr.unwrap_or_default()).into_owned();
        if !status.success() {
            let exit = match status.code() {
                Some(code) => format!("git exited with {code}"),
                None => "git was terminated by a signal".to_owned(),
            };
            return Err(GitQueryError::Failed {
                code: status.code(),
                reason: match first_line(&stderr) {
                    Some(line) => format!("{exit}: {line}"),
                    None => exit,
                },
            });
        }
        if let Some(warning) = first_line(&stderr) {
            tracing::debug!(warning, "git warning on a successful query");
        }
        written.map_err(|error| GitQueryError::failed(format!("git input failed: {error}")))?;
        stdout.map_err(|error| GitQueryError::failed(format!("git output failed: {error}")))
    })
}

/// Запуск `git`. На Windows он идёт в своей группе процессов консоли, как и прочие потомки
/// раннера: `CTRL_BREAK`, посланный группе раннера, достаётся только раннеру
/// (`INV.PLATFORM.A-CONSOLE-CHILD-IS-OUTSIDE-THE-RUNNER-PROCESS-GROUP`). На Unix группа не
/// меняется.
fn git() -> Command {
    let command = Command::new("git");
    #[cfg(windows)]
    let command = {
        let mut command = command;
        crate::platform::process::outside_the_runner_process_group(&mut command);
        command
    };
    command
}

/// `git` с каталогом и настройками, которые раннер задаёт каждому вопросу о рабочей копии:
/// без `GIT_OPTIONAL_LOCKS=0` git обновляет сведения о файлах в индексе, как обычный
/// `git status`; превью их не обновляет (`INV.CLI.PREVIEW-LEAVES-NO-TRACE`). Fsmonitor и
/// кэш неотслеживаемых раннер не включает: работает то, что настроено в репозитории.
/// Переменные, которые подменили бы репозиторий или индекс, из окружения раннера не
/// наследуются.
fn git_in(dir: &Path, optional_locks: bool) -> Command {
    let mut command = git();
    command.arg("-C").arg(dir);
    for variable in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"] {
        command.env_remove(variable);
    }
    if !optional_locks {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    command
}

/// Где лежит каталог набора: в рабочей копии, в подмодуле или вне git. Git, которого нет,
/// — вне рабочей копии.
pub fn placement_of(
    root: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Placement, GitQueryError> {
    if !root.is_dir() {
        return Ok(Placement::Outside(format!(
            "'{}' is not a directory",
            root.display()
        )));
    }
    let mut command = git_in(root, false);
    // Без `--path-format=absolute`: его нет у git старше 2.31, а путь от каталога вопроса
    // раннер дополняет сам.
    command.args([
        "rev-parse",
        "--git-path",
        "info/exclude",
        "--show-toplevel",
        "--show-superproject-working-tree",
    ]);
    let stdout = match run_git_query(command, None, OnCancel::Kill, interrupted) {
        Ok(stdout) => stdout,
        Err(GitQueryError::Interrupted) => return Err(GitQueryError::Interrupted),
        Err(GitQueryError::Unavailable(reason)) => return Ok(Placement::Outside(reason)),
        Err(GitQueryError::Failed { reason, .. }) => {
            return Ok(if inside_a_worktree(root) {
                Placement::Failed(reason)
            } else {
                Placement::Outside(reason)
            })
        }
    };
    let text = String::from_utf8_lossy(&stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let (Some(info_exclude), Some(toplevel)) = (lines.next(), lines.next()) else {
        // `--show-toplevel` молчит в каталоге `.git` и в голом репозитории.
        return Ok(Placement::Outside(format!(
            "'{}' is not in a working tree",
            root.display()
        )));
    };
    if lines.next().is_some() {
        return Ok(Placement::Submodule);
    }
    Ok(Placement::Inside {
        toplevel: PathBuf::from(toplevel),
        info_exclude: root.join(info_exclude),
    })
}

/// Путь от корня рабочей копии как pathspec git: буквально, без шаблонов.
fn literal_pathspec(relative: &str, exclude: bool) -> String {
    let magic = if exclude {
        "top,exclude,literal"
    } else {
        "top,literal"
    };
    if relative.is_empty() {
        format!(":({magic}).")
    } else {
        format!(":({magic}){relative}")
    }
}

/// Настройки и файлы правил игнорирования рабочей копии `toplevel` — без её файлов: что нужно,
/// чтобы проверить правила игнорирования, не спрашивая `git status`. Два вызова: `config` и
/// `ls-files` отслеживаемых `.gitignore`; неотслеживаемый `.gitignore` в каталогах над корнями
/// `roots` и в самих корнях раннер находит на диске.
pub fn ignore_view(
    toplevel: &Path,
    info_exclude: &Path,
    roots: &[String],
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<crate::domain::git_worktree::GitWorktree, GitQueryError> {
    use crate::domain::git_worktree::GitWorktree;

    // Ничего не пишут: необязательные блокировки не нужны.
    let mut config = git_in(toplevel, false);
    config.args([
        "config",
        "-z",
        "--type=path",
        "--get-regexp",
        r"^(core\.(fsmonitor|ignorecase|excludesfile)|extensions\.objectformat)$",
    ]);
    // Код 1 — ни одного ключа: настройки по умолчанию.
    let config = match run_git_query(config, None, OnCancel::Kill, interrupted) {
        Ok(stdout) => stdout,
        Err(GitQueryError::Failed { code: Some(1), .. }) => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut view = GitWorktree {
        toplevel: toplevel.to_path_buf(),
        ..GitWorktree::default()
    };
    let mut excludes_file = None;
    for record in config
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = String::from_utf8_lossy(record);
        let (key, value) = record.split_once('\n').unwrap_or((&record, "true"));
        match key {
            "core.fsmonitor" => view.fsmonitor = !is_false(value),
            "core.ignorecase" => view.ignore_case = !is_false(value),
            // Относительный путь git считает от корня рабочей копии.
            "core.excludesfile" => excludes_file = Some(toplevel.join(value)),
            "extensions.objectformat" => view.sha256 = value.trim().eq_ignore_ascii_case("sha256"),
            _ => {}
        }
    }

    let mut ls_files = git_in(toplevel, false);
    ls_files.args([
        "ls-files",
        "-z",
        "--full-name",
        "--",
        ":(top,glob)**/.gitignore",
    ]);
    let listed = run_git_query(ls_files, None, OnCancel::Kill, interrupted)?;
    let mut gitignores: Vec<String> = listed
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect();
    for root in roots {
        let mut dir = String::new();
        let mut candidates = vec![".gitignore".to_owned()];
        for part in root.split('/').filter(|part| !part.is_empty()) {
            dir = if dir.is_empty() {
                part.to_owned()
            } else {
                format!("{dir}/{part}")
            };
            candidates.push(format!("{dir}/.gitignore"));
        }
        gitignores.extend(
            candidates
                .into_iter()
                .filter(|path| toplevel.join(path).is_file()),
        );
    }
    view.ignore_sources = ignore_sources(toplevel, info_exclude, excludes_file, gitignores);
    Ok(view)
}

/// Файлы правил в порядке возрастания силы: `core.excludesFile`, `.git/info/exclude`, затем
/// `.gitignore` от мелких каталогов к глубоким.
fn ignore_sources(
    toplevel: &Path,
    info_exclude: &Path,
    excludes_file: Option<PathBuf>,
    mut gitignores: Vec<String>,
) -> Vec<crate::domain::git_worktree::GitIgnoreSource> {
    use crate::domain::git_worktree::GitIgnoreSource;

    gitignores.sort_by(|a, b| {
        a.matches('/')
            .count()
            .cmp(&b.matches('/').count())
            .then_with(|| a.cmp(b))
    });
    gitignores.dedup();
    let mut sources = Vec::new();
    if let Some(file) = excludes_file.or_else(default_excludes_file) {
        sources.push(GitIgnoreSource {
            file,
            base: String::new(),
        });
    }
    sources.push(GitIgnoreSource {
        file: info_exclude.to_path_buf(),
        base: String::new(),
    });
    sources.extend(gitignores.into_iter().map(|path| {
        GitIgnoreSource {
            file: toplevel.join(&path),
            base: path
                .rsplit_once('/')
                .map_or(String::new(), |(dir, _)| dir.to_owned()),
        }
    }));
    sources
}

/// Спрашивает git о файлах под `roots` рабочей копии `toplevel`: к [`ignore_view`] добавляются
/// `ls-files` — хеши и флаги индекса — и `status` — что разошлось с индексом и что вне его.
/// `excluded` — каталоги раннера под корнями (`workPath`, `localPath`), о которых не спрашивают. Пути — от корня
/// рабочей копии. `optional_locks` разрешает git обновить сведения о файлах в индексе; такой
/// `status` при отмене доходит до конца, чтобы не оставить `.git/index.lock`.
pub fn worktree_under(
    toplevel: &Path,
    info_exclude: &Path,
    roots: &[String],
    excluded: &[String],
    optional_locks: bool,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<crate::domain::git_worktree::GitWorktree, GitQueryError> {
    let started = std::time::Instant::now();
    let mut status = status_command(toplevel, optional_locks);
    status.args([
        "--porcelain=v2",
        "-z",
        "--untracked-files=all",
        "--no-renames",
        "--",
    ]);
    status.args(roots.iter().map(|root| literal_pathspec(root, false)));
    status.args(excluded.iter().map(|path| literal_pathspec(path, true)));
    let on_cancel = if optional_locks {
        OnCancel::Finish
    } else {
        OnCancel::Kill
    };
    // `git status` — самый долгий вопрос — идёт рядом с остальными (#645): они только читают
    // индекс и настройки, а сведения о файлах, которые обновляет `status`, хешей индекса не
    // меняют. Индекс читается первым: `status` заменяет его в самом конце, и на Windows замена
    // файла, который ещё читают, откладывается. Отмену ему передаёт этот поток.
    let stop = std::sync::atomic::AtomicBool::new(false);
    let (others, answer) = std::thread::scope(|scope| {
        let stop = &stop;
        let status = scope.spawn(move || {
            let mut stopped = || stop.load(std::sync::atomic::Ordering::Relaxed);
            let answer = run_git_query(status, None, on_cancel, &mut stopped);
            (answer, started.elapsed().as_millis() as u64)
        });
        let others = index_entries_under(toplevel, roots, interrupted).and_then(|index| {
            let index_ms = started.elapsed().as_millis() as u64;
            let view = ignore_view(toplevel, info_exclude, roots, interrupted)?;
            Ok((view, index, started.elapsed().as_millis() as u64, index_ms))
        });
        if others.is_err() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        while !status.is_finished() {
            if interrupted() {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            std::thread::sleep(GIT_INTERRUPTION_POLL);
        }
        let answer = status.join().unwrap_or_else(|_| {
            (
                Err(GitQueryError::failed("a thread asking git panicked")),
                0,
            )
        });
        (others, answer)
    });
    let (mut view, index, ignore_ms, index_ms) = others?;
    let (answer, status_ms) = answer;
    view.index = index;
    (view.worktree_changed, view.untracked) = parse_status_v2(&answer?)?;
    tracing::debug!(
        toplevel = %toplevel.display(),
        index = view.index.len(),
        changed = view.worktree_changed.len(),
        untracked = view.untracked.len(),
        ignore_ms,
        index_ms,
        status_ms,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "git described the working tree"
    );
    // Неотслеживаемый `.gitignore` внутри набора git применяет так же, как отслеживаемый.
    let untracked_gitignores: Vec<String> = view
        .untracked
        .iter()
        .filter(|path| path.rsplit('/').next() == Some(".gitignore"))
        .cloned()
        .collect();
    if !untracked_gitignores.is_empty() {
        let mut gitignores: Vec<String> = view
            .ignore_sources
            .iter()
            .filter(|source| source.file.file_name() == Some(std::ffi::OsStr::new(".gitignore")))
            .filter_map(|source| {
                source
                    .file
                    .strip_prefix(toplevel)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            })
            .collect();
        gitignores.extend(untracked_gitignores);
        let excludes = view
            .ignore_sources
            .first()
            .filter(|source| source.file != info_exclude)
            .map(|source| source.file.clone());
        view.ignore_sources = ignore_sources(toplevel, info_exclude, excludes, gitignores);
    }
    Ok(view)
}

/// Записи индекса под `roots`: `git ls-files -s -v`.
pub fn index_entries_under(
    toplevel: &Path,
    roots: &[String],
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<crate::domain::git_worktree::GitIndexEntry>, GitQueryError> {
    let mut ls_files = git_in(toplevel, false);
    ls_files.args(["ls-files", "-s", "-v", "-z", "--full-name", "--"]);
    ls_files.args(roots.iter().map(|root| literal_pathspec(root, false)));
    parse_ls_files(&run_git_query(ls_files, None, OnCancel::Kill, interrupted)?)
}

/// `git status` с доверием отметкам времени, как его задаёт раннер (#575): для файла, который
/// git считает неизменным по размеру и времени, раннер верит git.
fn status_command(toplevel: &Path, optional_locks: bool) -> Command {
    let mut status = git_in(toplevel, optional_locks);
    status.args([
        "-c",
        "core.checkStat=default",
        "-c",
        "core.trustctime=true",
        "status",
    ]);
    status
}

/// Хеши blob, которые сохранил бы `git add` для файлов `paths` (от корня рабочей копии), в их
/// порядке. Считает их сам git во временном индексе: `known` — записи настоящего индекса для
/// этих путей, и преобразование концов строк видит то, что лежит в индексе, как у
/// `git add` (#575). Объектов в хранилище и настоящего индекса вопрос не пишет.
pub fn blob_ids_as_added(
    toplevel: &Path,
    paths: &[String],
    known: &[&crate::domain::git_worktree::GitIndexEntry],
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<String>, GitQueryError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let temp = tempfile::tempdir()
        .map_err(|error| GitQueryError::failed(format!("temporary index: {error}")))?;
    let index = temp.path().join("index");
    let in_temp_index = || {
        let mut command = git_in(toplevel, false);
        command.env("GIT_INDEX_FILE", &index).args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.splitIndex=false",
            "-c",
            "core.untrackedCache=false",
        ]);
        command
    };
    if !known.is_empty() {
        let mut info = Vec::new();
        for entry in known {
            info.extend_from_slice(
                format!(
                    "{:06o} {} {}\t{}",
                    entry.mode, entry.oid, entry.stage, entry.path
                )
                .as_bytes(),
            );
            info.push(0);
        }
        let mut command = in_temp_index();
        command.args(["update-index", "-z", "--index-info"]);
        run_git_query(command, Some(info), OnCancel::Kill, interrupted)?;
    }
    let mut listed = Vec::new();
    for path in paths {
        listed.extend_from_slice(path.as_bytes());
        listed.push(0);
    }
    let mut command = in_temp_index();
    command.args([
        "update-index",
        "--add",
        "--remove",
        "--info-only",
        "-z",
        "--stdin",
    ]);
    run_git_query(command, Some(listed), OnCancel::Kill, interrupted)?;
    let mut command = in_temp_index();
    command.args(["ls-files", "-s", "-v", "-z", "--full-name"]);
    let entries = parse_ls_files(&run_git_query(command, None, OnCancel::Kill, interrupted)?)?;
    let ids: std::collections::HashMap<&str, &str> = entries
        .iter()
        .filter(|entry| entry.stage == 0)
        .map(|entry| (entry.path.as_str(), entry.oid.as_str()))
        .collect();
    paths
        .iter()
        .map(|path| {
            ids.get(path.as_str())
                .map(|id| (*id).to_owned())
                .ok_or_else(|| GitQueryError::failed(format!("git did not hash '{path}'")))
        })
        .collect()
}

fn is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "no" | "off" | "0" | ""
    )
}

/// `core.excludesFile` по умолчанию: `$XDG_CONFIG_HOME/git/ignore`, иначе
/// `~/.config/git/ignore`.
fn default_excludes_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("git").join("ignore"))
}

/// Разбирает `git ls-files -s -v -z`: `<метка> <вид> <хеш> <ступень>\t<путь>`.
fn parse_ls_files(
    stdout: &[u8],
) -> Result<Vec<crate::domain::git_worktree::GitIndexEntry>, GitQueryError> {
    let malformed =
        |record: &str| GitQueryError::failed(format!("unexpected git ls-files record '{record}'"));
    let mut entries = Vec::new();
    for record in stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = std::str::from_utf8(record)
            .map_err(|_| GitQueryError::failed("git ls-files named a path that is not UTF-8"))?;
        let (head, path) = record.split_once('\t').ok_or_else(|| malformed(record))?;
        let mut fields = head.split(' ');
        let (Some(tag), Some(mode), Some(oid), Some(stage)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(malformed(record));
        };
        let tag = tag.chars().next().ok_or_else(|| malformed(record))?;
        entries.push(crate::domain::git_worktree::GitIndexEntry {
            path: path.to_owned(),
            mode: u32::from_str_radix(mode, 8).map_err(|_| malformed(record))?,
            oid: oid.to_owned(),
            stage: stage.parse().map_err(|_| malformed(record))?,
            flagged: tag.is_ascii_lowercase() || tag == 'S',
        });
    }
    Ok(entries)
}

/// Разбирает `git status --porcelain=v2 -z --no-renames`: пути, у которых рабочее дерево
/// разошлось с индексом, и файлы вне индекса.
fn parse_status_v2(
    stdout: &[u8],
) -> Result<(std::collections::HashSet<String>, Vec<String>), GitQueryError> {
    let mut changed = std::collections::HashSet::new();
    let mut untracked = Vec::new();
    let mut records = stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let record = std::str::from_utf8(record)
            .map_err(|_| GitQueryError::failed("git status named a path that is not UTF-8"))?;
        let malformed =
            || GitQueryError::failed(format!("unexpected git status record '{record}'"));
        match record.as_bytes()[0] {
            b'1' => {
                let fields: Vec<&str> = record.splitn(9, ' ').collect();
                let (Some(xy), Some(path)) = (fields.get(1), fields.get(8)) else {
                    return Err(malformed());
                };
                if xy.as_bytes().get(1) != Some(&b'.') {
                    changed.insert((*path).to_owned());
                }
            }
            b'2' => {
                // Переименований `--no-renames` не даёт; запись разобрана на случай чужой
                // настройки: за ней идёт отдельным полем прежнее имя.
                let fields: Vec<&str> = record.splitn(10, ' ').collect();
                let path = fields.get(9).ok_or_else(malformed)?;
                changed.insert((*path).to_owned());
                if let Some(original) = records.next() {
                    changed.insert(String::from_utf8_lossy(original).into_owned());
                }
            }
            b'u' => {
                let fields: Vec<&str> = record.splitn(11, ' ').collect();
                changed.insert((*fields.get(10).ok_or_else(malformed)?).to_owned());
            }
            b'?' => untracked.push(record.get(2..).ok_or_else(malformed)?.to_owned()),
            // Игнорируемых без `--ignored` нет; заголовков без `--branch` тоже.
            b'!' | b'#' => {}
            _ => return Err(malformed()),
        }
    }
    Ok((changed, untracked))
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

    /// Отмена во время вопроса к git завершает git, и ответ — отмена, а не отказ git, после
    /// которого раннер читал бы файлы сам (`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`).
    #[test]
    fn a_cancelled_git_query_ends_git_and_answers_interrupted() {
        let repo = repo_with_committed_source();
        let mut command = git_in(repo.path(), false);
        command.args(["-c", "alias.wait=!sleep 3", "wait"]);
        let mut asked = 0;
        let started = std::time::Instant::now();
        let answer = run_git_query(command, None, OnCancel::Kill, &mut || {
            asked += 1;
            asked > 1
        });
        assert_eq!(answer, Err(GitQueryError::Interrupted));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the query did not wait for git to finish on its own"
        );
    }

    /// Ответ git о рабочей копии: хеш индекса, флаги, правка, файл вне индекса и отслеживаемый
    /// `.gitignore` как источник правил.
    #[test]
    fn the_worktree_answer_is_parsed_from_ls_files_and_status() {
        let repo = repo_with_committed_source();
        fs::write(repo.path().join(".gitignore"), "/.work/\n").expect("gitignore");
        run_git(repo.path(), &["add", ".gitignore"]);
        run_git(repo.path(), &["commit", "-qm", "rules"]);
        fs::write(source_dir(&repo).join("Configuration.xml"), "edited\n").expect("edit");
        fs::write(source_dir(&repo).join("New.xml"), "new\n").expect("new");
        let (toplevel, info_exclude) = match placement_of(&source_dir(&repo), &mut || false) {
            Ok(Placement::Inside {
                toplevel,
                info_exclude,
            }) => (toplevel, info_exclude),
            other => panic!("expected a working tree, got {other:?}"),
        };

        let view = worktree_under(
            &toplevel,
            &info_exclude,
            &["src/cf".to_owned()],
            &[],
            false,
            &mut || false,
        )
        .expect("answer");
        assert_eq!(view.index.len(), 1);
        assert_eq!(view.index[0].path, "src/cf/Configuration.xml");
        assert_eq!(view.index[0].oid.len(), 40);
        assert!(view.worktree_changed.contains("src/cf/Configuration.xml"));
        assert_eq!(view.untracked, ["src/cf/New.xml"]);
        assert!(view
            .ignore_sources
            .iter()
            .any(|source| source.file.ends_with(".gitignore") && source.base.is_empty()));
        let ids = blob_ids_as_added(&toplevel, &["src/cf/New.xml".to_owned()], &[], &mut || {
            false
        })
        .expect("ids");
        assert_eq!(ids.len(), 1);
    }

    /// `git status`, которому разрешено обновить индекс, при отмене доходит до конца:
    /// принудительно завершённый, он оставил бы `.git/index.lock`, и следующая команда git в
    /// рабочей копии отказала бы.
    #[test]
    fn a_cancelled_status_leaves_no_index_lock() {
        let repo = repo_with_committed_source();
        fs::write(source_dir(&repo).join("Configuration.xml"), "conf\n").expect("same bytes");
        let (toplevel, info_exclude) = match placement_of(&source_dir(&repo), &mut || false) {
            Ok(Placement::Inside {
                toplevel,
                info_exclude,
            }) => (toplevel, info_exclude),
            other => panic!("expected a working tree, got {other:?}"),
        };
        // Отмена приходит, когда git status уже запущен: раньше спрашивают config и ls-files.
        let mut asked = 0;
        let answer = worktree_under(
            &toplevel,
            &info_exclude,
            &["src/cf".to_owned()],
            &[],
            true,
            &mut || {
                asked += 1;
                asked > 4
            },
        );
        assert_eq!(answer.err(), Some(GitQueryError::Interrupted));
        assert!(
            !repo.path().join(".git").join("index.lock").exists(),
            "a cancelled status left the index locked"
        );
    }

    /// Хеш, который сохранил бы `git add`: файл, закоммиченный с CRLF до `core.autocrlf=true`,
    /// git не нормализует, и `hash-object` без индекса дал бы другой хеш (#575).
    #[test]
    fn the_id_is_the_one_git_add_would_store() {
        let repo = repo_with_committed_source();
        let module = source_dir(&repo).join("Module.bsl");
        fs::write(&module, "a\r\nb\r\n").expect("crlf");
        run_git(repo.path(), &["add", "-A"]);
        run_git(repo.path(), &["commit", "-qm", "crlf"]);
        run_git(repo.path(), &["config", "core.autocrlf", "true"]);
        fs::write(&module, "a\r\nb\r\nc\r\n").expect("edit");
        let toplevel = repo.path().canonicalize().expect("canonical");
        let path = "src/cf/Module.bsl".to_owned();
        let entries = index_entries_under(&toplevel, std::slice::from_ref(&path), &mut || false)
            .expect("index");
        let known: Vec<_> = entries.iter().collect();

        let id = blob_ids_as_added(&toplevel, std::slice::from_ref(&path), &known, &mut || {
            false
        })
        .expect("id")
        .remove(0);

        let index = fs::read(repo.path().join(".git").join("index")).expect("index");
        run_git(repo.path(), &["add", "src/cf/Module.bsl"]);
        let added = index_entries_under(&toplevel, std::slice::from_ref(&path), &mut || false)
            .expect("index");
        assert_eq!(id, added[0].oid);
        fs::write(repo.path().join(".git").join("index"), index).expect("restore");
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
        let fired = repo.path().join("hook-fired");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\ntouch '{}'\necho 'warning: in the working copy of x, LF will be replaced by CRLF' >&2\nexit 1\n",
                fired.display()
            ),
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
        // Без метки хук не сработал (старый git без `core.fsmonitor`-хука), и предупреждения
        // не было — тест прошёл бы впустую.
        assert!(fired.is_file(), "the fsmonitor hook must have run");
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
