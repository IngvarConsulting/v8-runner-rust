//! Сторож: не дать замене или перезаписи каталога уничтожить работу, которую не вернуть.
//!
//! Платформа не знает о том, что человек держит в каталоге исходников: замена каталога
//! стирает оттуда всё лишнее, а выгрузка поверх него переписывает файлы на месте.
//!
//! Спрашивают об этом систему контроля версий, и ответов у неё три: терять нечего, есть
//! безвозвратное, ответа нет. Третий ответ — гита нет, каталог вне рабочей копии, вызов не
//! удался — приравнен ко второму: потерять можно всё, что в каталоге лежит, и отказ идёт
//! на тех же правах, с перечнем каждого файла. Пустой каталог терять нечего ни в каком
//! случае.
//!
//! Согласие (`--force`) пропускает работу и называет уничтоженное: то, что нашла система
//! контроля версий, или весь каталог, когда ответа нет.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::platform::git::{uncommitted_work_in, UncommittedWork};
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, ExecutionTransport};

/// Сколько потерь перечислять в тексте, прежде чем считать их числом. Полный перечень
/// несёт поле ответа.
const NAMED_LOSS_LIMIT: usize = 20;

/// Чьё содержимое лежит в каталоге и разрешено ли его уничтожить.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DestructionConsent {
    /// Каталог раннер завёл для себя: кеш инструментов и тому подобное. Спрашивать
    /// систему контроля версий не о чем.
    RunnerOwned,
    /// Каталог назвал человек. Безвозвратное останавливает работу, и отказ называет
    /// выходы, которые у вызывающего есть.
    AskFirst(WaysOut),
    /// Человек попросил уничтожить явно.
    Granted,
}

/// Выходы из отказа, кроме общего для всех «сохранить работу и повторить». Их называет
/// вызывающий: только он знает, есть ли у его цели замена в командной строке и какая.
///
/// Это не повтор [`ForceWayOut`](crate::use_cases::request::ForceWayOut). Тот — поле
/// запроса `pull`, которое транспорт заполняет, не зная, какой набор разрешится: он
/// говорит только, можно ли слать вызывающего к замене. Здесь выход уже собран сценарием
/// для своей цели: с разрешённым набором для `pull` и с вариантом `convert`, у которого
/// запроса `pull` нет вовсе.
///
/// Совет, выполненный буквально, обязан бить в ту же цель и не упираться во второй отказ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WaysOut {
    /// Только сохранить работу: заменить каталог вызывающему нечем (`clone`).
    SaveWork,
    /// Ещё повторить тот же вызов с добавленным `--force`. Годится, где `--force` ни с чем
    /// в вызове не спорит (`convert`).
    SameCallWithForce,
    /// Ещё полностью заменить каталог набора точной командой `pull <SET> --force` с
    /// глобальными ключами запуска. Не «тот же вызов с `--force`»: рядом с `--object` и
    /// прежним `--mode` ключ отказывает, и буквальный повтор упёрся бы во второй отказ.
    PullForce {
        /// Набор так, как его принимает позиционный аргумент командной строки.
        source_set: String,
    },
}

/// Что работа делает с каталогом — так её и называет отказ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Destruction {
    /// Каталог заменяется целиком.
    Replace,
    /// Выгрузка ложится поверх каталога и переписывает файлы в нём.
    Overwrite,
}

impl Destruction {
    fn verb(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Overwrite => "overwrite",
        }
    }
}

/// Что в каталоге пропадёт безвозвратно.
///
/// Пути — такими, какими их называет ответ: от корня рабочей копии, когда гит ответил
/// (так их не спутать между наборами), и полными, когда ответа нет и корня тоже.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Losses {
    paths: Vec<PathBuf>,
    /// Почему система контроля версий не ответила. Тогда потеря — каждый файл каталога.
    unanswered: Option<String>,
}

impl Losses {
    pub(super) fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub(super) fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }

    /// Перечень для текста: первые имена и счёт остальных.
    fn named(&self) -> String {
        let named: Vec<String> = self
            .paths
            .iter()
            .take(NAMED_LOSS_LIMIT)
            .map(|path| path.display().to_string())
            .collect();
        let rest = self.paths.len().saturating_sub(named.len());
        let tail = if rest > 0 {
            format!(", and {rest} more")
        } else {
            String::new()
        };
        format!("{}{tail}", named.join(", "))
    }

    /// Что потеряется в каталоге, названном рядом, — одной фразой.
    fn describe(&self, tense: Tense) -> String {
        let count = self.paths.len();
        let (exist, gives) = match tense {
            Tense::Present => ("exist", "gives"),
            Tense::Past => ("existed", "gave"),
        };
        match &self.unanswered {
            None => format!("{count} file(s) there {exist} nowhere else ({})", self.named()),
            Some(reason) => format!(
                "version control {gives} no answer there ({reason}), so all {count} file(s) in it {exist} nowhere else ({})",
                self.named()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Tense {
    Present,
    Past,
}

/// Спрашивает, что в `target` не восстановить. Ничего не пишет и согласия не учитывает.
///
/// `regenerated` — имена файлов в корне `target`, которые работа пишет заново; их
/// называет вызывающий, потому что только он знает, что пишет. Выгрузка в формате
/// Конфигуратора передаёт опись версий: платформа пишет её в каждую выгрузку, а штатно
/// опись лежит в игноре, и без исключения отказ стоял бы на каждой выгрузке.
/// Преобразование и замена проекта EDT описи не пишут — у них исключений нет.
pub(super) fn losses_in(target: &Path, regenerated: &[&str]) -> Losses {
    match uncommitted_work_in(target, regenerated) {
        // Терять нечего: прежнее содержимое система контроля версий вернёт сама.
        UncommittedWork::Nothing => Losses::default(),
        UncommittedWork::AtRisk(paths) => Losses {
            paths,
            unanswered: None,
        },
        // Ответа нет — потерять можно всё, что лежит в каталоге.
        UncommittedWork::Unknown(reason) => {
            let paths = every_file_in(target, regenerated);
            Losses {
                unanswered: (!paths.is_empty()).then_some(reason),
                paths,
            }
        }
    }
}

/// Каждый файл под `target`, кроме названных в `regenerated` в его корне. Каталоги сами по
/// себе потерей не считаются; по ссылкам обход не идёт. То, что прочесть не удалось,
/// называется своим путём: о его содержимом ответа нет, а терять его так же можно.
fn every_file_in(target: &Path, regenerated: &[&str]) -> Vec<PathBuf> {
    WalkDir::new(target)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_dir() => None,
            Ok(entry)
                if entry.depth() == 1
                    && regenerated
                        .iter()
                        .any(|name| entry.file_name() == std::ffi::OsStr::new(name)) =>
            {
                None
            }
            Ok(entry) => Some(entry.into_path()),
            Err(error) => Some(error.path().unwrap_or(target).to_path_buf()),
        })
        .collect()
}

/// Отказывает до того, как что-либо стёрто, либо пропускает работу дальше и отдаёт, что
/// она уничтожит по согласию; без согласия отдаёт пустое.
///
/// `context` называет транспорт и глобальные ключи запуска: повторить вызов человек
/// командной строки и клиент MCP могут по-разному, а команда в совете должна попасть в ту
/// же цель.
pub(super) fn guard_replacement(
    context: &ExecutionContext,
    target: &Path,
    consent: &DestructionConsent,
    regenerated: &[&str],
    how: Destruction,
) -> Result<Losses, AppError> {
    if *consent == DestructionConsent::RunnerOwned {
        return Ok(Losses::default());
    }
    let losses = losses_in(target, regenerated);
    match consent {
        DestructionConsent::AskFirst(ways_out) if !losses.is_empty() => Err(AppError::Validation(
            refusal(target, &losses, ways_out, context, how),
        )),
        // Попросили уничтожить — уничтожаем, как и обещает имя ключа, и называем что.
        DestructionConsent::AskFirst(_)
        | DestructionConsent::Granted
        | DestructionConsent::RunnerOwned => Ok(losses),
    }
}

/// Строка ответа о том, что работа уничтожила по согласию; `None`, если ничего.
pub(super) fn discard_note(target: &Path, losses: &Losses) -> Option<String> {
    (!losses.is_empty()).then(|| {
        format!(
            "discarded on request in '{}': {}",
            target.display(),
            losses.describe(Tense::Past)
        )
    })
}

/// Строка превью о потерях: на чём работа остановится без согласия или что она уничтожит
/// с ним; `None`, если терять нечего.
pub(super) fn preview_note(
    context: &ExecutionContext,
    target: &Path,
    consent: &DestructionConsent,
    losses: &Losses,
    how: Destruction,
) -> Option<String> {
    if losses.is_empty() {
        return None;
    }
    match consent {
        DestructionConsent::AskFirst(ways_out) => Some(format!(
            "it would stop before the platform starts: {}",
            refusal(target, losses, ways_out, context, how)
        )),
        DestructionConsent::Granted => Some(format!(
            "it would discard in '{}': {}",
            target.display(),
            losses.describe(Tense::Present)
        )),
        DestructionConsent::RunnerOwned => None,
    }
}

fn refusal(
    target: &Path,
    losses: &Losses,
    ways_out: &WaysOut,
    context: &ExecutionContext,
    how: Destruction,
) -> String {
    // Одной строкой: человеческий вывод — закреплённая форма, и многострочная
    // подробность в нём рассыпается по разным видам строк.
    format!(
        "refusing to {} '{}': {}; {}",
        how.verb(),
        target.display(),
        losses.describe(Tense::Present),
        remedy(ways_out, context, losses.unanswered.is_some())
    )
}

/// Выход из отказа, который у вызывающего есть.
///
/// Готовой команды, урезанной до имени команды, отказ не собирает: без набора, каталога
/// вывода и глобальных ключей буквальный повтор бьёт в чужой каталог или чужую базу.
/// Точная команда `pull` несёт набор и глобальные ключи запуска; у MCP по HTTP она
/// исполнима только там, где работает сервер.
///
/// Без ответа системы контроля версий «закоммитить» нечем: работу сохраняют, взяв каталог
/// под контроль версий или унеся файлы.
fn remedy(ways_out: &WaysOut, context: &ExecutionContext, unanswered: bool) -> String {
    const DISCARDS: &str = "which replaces the directory and discards them";
    let transport = context.transport();
    let keep = if unanswered {
        "put them under version control and commit them, or move them away,"
    } else {
        "commit or stash them"
    };
    let again = match transport {
        ExecutionTransport::Cli => "run the same command again",
        ExecutionTransport::McpStdio | ExecutionTransport::McpHttp => "call the tool again",
    };
    let save = format!("{keep} and {again}");
    match ways_out {
        WaysOut::SaveWork => save,
        WaysOut::SameCallWithForce => match transport {
            ExecutionTransport::Cli => {
                format!("{save}, or repeat the same command with `--force` added, {DISCARDS}")
            }
            // Преобразования у MCP нет; если появится, точной команды здесь собрать не из
            // чего, и совет не подставляет урезанную.
            ExecutionTransport::McpStdio | ExecutionTransport::McpHttp => format!(
                "{save}, or run the matching `v8-runner` command with `--force` for the same target from the command line, {DISCARDS}"
            ),
        },
        WaysOut::PullForce { source_set } => {
            let command = context.advised_pull_force(source_set);
            format!(
                "{save}, or run {command}: a full dump of source-set '{source_set}' that replaces its whole directory and discards them"
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_git::init_git_repo;
    use crate::use_cases::ignored_files::VERSION_FILE_NAME;
    use std::fs;
    use tempfile::tempdir;

    use crate::use_cases::context::{AdvisedInfobase, CommandLineTarget, CommandName};

    fn cli() -> ExecutionContext {
        ExecutionContext::cli(CommandName::Dump)
    }

    fn pull_force(source_set: &str) -> WaysOut {
        WaysOut::PullForce {
            source_set: source_set.to_owned(),
        }
    }

    fn ask_first() -> DestructionConsent {
        DestructionConsent::AskFirst(pull_force("main"))
    }

    /// Потери, которые нашёл гит.
    fn answered(paths: &[PathBuf]) -> Losses {
        Losses {
            paths: paths.to_vec(),
            unanswered: None,
        }
    }

    fn cli_refusal(target: &Path, paths: &[PathBuf]) -> String {
        refusal(
            target,
            &answered(paths),
            &WaysOut::SameCallWithForce,
            &cli(),
            Destruction::Replace,
        )
    }

    /// Сервер, запущенный с конфигом в другом каталоге, с невыбранной по умолчанию базой
    /// и переопределённым рабочим каталогом.
    fn started_elsewhere() -> CommandLineTarget {
        CommandLineTarget {
            config: Some(PathBuf::from("/srv/my project/v8project.yaml")),
            infobase: Some(AdvisedInfobase::Name("staging".to_owned())),
            workdir: Some(PathBuf::from("/var/tmp/v8w")),
        }
    }

    #[test]
    fn a_runner_owned_directory_is_never_questioned() {
        let dir = tempdir().expect("tempdir");
        assert!(guard_replacement(
            &cli(),
            dir.path(),
            &DestructionConsent::RunnerOwned,
            &[],
            Destruction::Replace
        )
        .is_ok());
    }

    /// Вне репозитория ответа нет: потерять можно всё. Отказ идёт на тех же правах, что
    /// найденное безвозвратное, и называет каждый файл, кроме того, что работа пишет заново.
    #[test]
    fn without_an_answer_every_file_is_a_loss_and_the_work_is_refused() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("hand-written.xml"), "mine\n").expect("write");
        fs::create_dir_all(dir.path().join("Catalogs").join("Empty")).expect("dirs");
        fs::write(dir.path().join("Catalogs").join("Item.xml"), "mine\n").expect("write");
        fs::write(dir.path().join(VERSION_FILE_NAME), "<info/>\n").expect("version file");

        let Err(AppError::Validation(message)) = guard_replacement(
            &cli(),
            dir.path(),
            &ask_first(),
            &[VERSION_FILE_NAME],
            Destruction::Overwrite,
        ) else {
            panic!("a directory without an answer must be refused");
        };
        assert!(message.contains("refusing to overwrite"), "{message}");
        assert!(
            message.contains("version control gives no answer"),
            "{message}"
        );
        assert!(message.contains("all 2 file(s)"), "{message}");
        assert!(message.contains("hand-written.xml"), "{message}");
        assert!(message.contains("Item.xml"), "{message}");
        assert!(!message.contains(VERSION_FILE_NAME), "{message}");
        assert!(
            message.contains("put them under version control and commit them"),
            "{message}"
        );
    }

    /// Пустой каталог терять нечего, есть у него ответ или нет.
    #[test]
    fn an_empty_directory_without_an_answer_has_nothing_to_lose() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("nested")).expect("nested");
        fs::write(dir.path().join(VERSION_FILE_NAME), "<info/>\n").expect("version file");
        let losses = guard_replacement(
            &cli(),
            dir.path(),
            &ask_first(),
            &[VERSION_FILE_NAME],
            Destruction::Replace,
        )
        .expect("nothing to lose");
        assert!(losses.is_empty(), "{losses:?}");
    }

    /// Согласие называет уничтоженное: найденное гитом и весь каталог без ответа.
    #[test]
    fn consent_names_what_it_destroys() {
        let outside = tempdir().expect("tempdir");
        fs::write(outside.path().join("hand-written.xml"), "mine\n").expect("write");
        let losses = guard_replacement(
            &cli(),
            outside.path(),
            &DestructionConsent::Granted,
            &[],
            Destruction::Replace,
        )
        .expect("consent proceeds");
        assert_eq!(
            losses.clone().into_paths(),
            vec![outside.path().join("hand-written.xml")]
        );
        let note = discard_note(outside.path(), &losses).expect("a note");
        assert!(note.contains("version control gave no answer"), "{note}");
        assert!(note.contains("hand-written.xml"), "{note}");

        let repo = tempdir().expect("tempdir");
        init_git_repo(repo.path());
        fs::write(repo.path().join("hand-written.xml"), "mine\n").expect("write");
        let losses = guard_replacement(
            &cli(),
            repo.path(),
            &DestructionConsent::Granted,
            &[],
            Destruction::Replace,
        )
        .expect("consent proceeds");
        assert_eq!(
            losses.clone().into_paths(),
            vec![PathBuf::from("hand-written.xml")]
        );
        let note = discard_note(repo.path(), &losses).expect("a note");
        assert!(!note.contains("no answer"), "{note}");
    }

    /// Превью называет потери, ничего не трогая: без согласия — отказ, который случится, с
    /// согласием — уничтожение.
    #[test]
    fn a_preview_names_the_losses_for_either_consent() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("hand-written.xml"), "mine\n").expect("write");
        let losses = losses_in(dir.path(), &[]);

        let refused = preview_note(
            &cli(),
            dir.path(),
            &ask_first(),
            &losses,
            Destruction::Overwrite,
        )
        .expect("a note");
        assert!(refused.contains("would stop"), "{refused}");
        assert!(refused.contains("refusing to overwrite"), "{refused}");
        let granted = preview_note(
            &cli(),
            dir.path(),
            &DestructionConsent::Granted,
            &losses,
            Destruction::Replace,
        )
        .expect("a note");
        assert!(granted.contains("would discard"), "{granted}");
        assert!(granted.contains("hand-written.xml"), "{granted}");
        assert!(dir.path().join("hand-written.xml").is_file());
        assert_eq!(
            preview_note(
                &cli(),
                dir.path(),
                &ask_first(),
                &Losses::default(),
                Destruction::Replace
            ),
            None
        );
    }

    #[test]
    fn the_refusal_names_what_would_be_lost() {
        let message = cli_refusal(
            Path::new("/project/src/cf"),
            &[PathBuf::from("src/cf/hand-written.xml")],
        );
        assert!(message.contains("src/cf/hand-written.xml"), "{message}");
    }

    /// Отказ называет выходы, которые у вызывающего есть: сохранить работу и повторить —
    /// всегда, а заменить каталог — только тому, кому это доступно.
    #[test]
    fn the_refusal_names_the_ways_out_the_caller_has() {
        let lost = [PathBuf::from("src/cf/hand-written.xml")];
        let target = Path::new("/project/src/cf");

        let same_call = refusal(
            target,
            &answered(&lost),
            &WaysOut::SameCallWithForce,
            &cli(),
            Destruction::Replace,
        );
        assert!(
            same_call.contains("commit or stash them and run the same command again"),
            "{same_call}"
        );
        assert!(
            same_call.contains("repeat the same command with `--force` added"),
            "{same_call}"
        );

        let pull = refusal(
            target,
            &answered(&lost),
            &pull_force("ext"),
            &cli(),
            Destruction::Replace,
        );
        assert!(
            pull.contains("commit or stash them and run the same command again"),
            "{pull}"
        );
        assert!(pull.contains("pull ext --force`"), "{pull}");

        for context in [
            cli(),
            ExecutionContext::mcp_stdio(CommandName::Dump),
            ExecutionContext::mcp_http(CommandName::Dump),
        ] {
            let save_only = refusal(
                target,
                &answered(&lost),
                &WaysOut::SaveWork,
                &context,
                Destruction::Replace,
            );
            assert!(save_only.contains("commit or stash them"), "{save_only}");
            assert!(!save_only.contains("--force"), "{save_only}");
        }

        for context in [
            ExecutionContext::mcp_stdio(CommandName::Dump),
            ExecutionContext::mcp_http(CommandName::Dump),
        ] {
            let mcp = refusal(
                target,
                &answered(&lost),
                &pull_force("ext"),
                &context,
                Destruction::Replace,
            );
            assert!(
                mcp.contains("commit or stash them and call the tool again"),
                "{mcp}"
            );
            assert!(
                mcp.contains("pull ext --force` from the command line"),
                "{mcp}"
            );
            assert!(!mcp.contains("pass --force"), "{mcp}");
        }
    }

    /// Совет повторяет исходную цель: набор и глобальные ключи запуска. Буквально
    /// выполненный из другого каталога, он бьёт в тот же проект, ту же базу и тот же
    /// рабочий каталог, а не в базу по умолчанию соседнего проекта.
    #[test]
    fn the_pull_advice_repeats_the_target_with_the_global_keys_of_the_run() {
        let lost = [PathBuf::from("src/cf/hand-written.xml")];
        let target = Path::new("/project/src/cf");
        let expected = "`v8-runner --config '/srv/my project/v8project.yaml' --infobase staging --workdir /var/tmp/v8w pull ext --force`";

        for context in [
            ExecutionContext::cli(CommandName::Dump),
            ExecutionContext::mcp_stdio(CommandName::Dump),
            ExecutionContext::mcp_http(CommandName::Dump),
        ] {
            let transport = context.transport();
            let context = context.with_command_line(started_elsewhere());
            let message = refusal(
                target,
                &answered(&lost),
                &pull_force("ext"),
                &context,
                Destruction::Replace,
            );
            assert!(message.contains(expected), "{transport:?}: {message}");
            assert!(
                message
                    .contains("a full dump of source-set 'ext' that replaces its whole directory and discards them"),
                "{transport:?}: {message}"
            );
            assert_eq!(
                message.contains("on the machine where the MCP server runs"),
                transport == ExecutionTransport::McpHttp,
                "{transport:?}: {message}"
            );
        }
    }

    /// Попросили явно — уничтожаем, как и обещает имя ключа.
    #[test]
    fn an_explicit_request_discards_instead_of_hoarding() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert!(guard_replacement(
            &cli(),
            root,
            &DestructionConsent::Granted,
            &[],
            Destruction::Replace
        )
        .is_ok());
        assert!(matches!(
            guard_replacement(&cli(), root, &ask_first(), &[], Destruction::Replace),
            Err(AppError::Validation(_))
        ));
    }

    /// Опись версий в корне замена пишет заново: в игноре она не повод отказать.
    #[test]
    fn an_ignored_version_file_at_the_root_is_not_a_loss() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::write(root.join(".gitignore"), format!("{VERSION_FILE_NAME}\n")).expect("ignore");

        let asked = root.join("cf");
        fs::create_dir_all(&asked).expect("cf");
        fs::write(asked.join(VERSION_FILE_NAME), "<info/>\n").expect("version file");
        assert!(guard_replacement(
            &cli(),
            &asked,
            &ask_first(),
            &[VERSION_FILE_NAME],
            Destruction::Replace
        )
        .is_ok());
    }

    /// Замена, которая опись не пишет (преобразование, проект EDT), не вправе
    /// считать её восстановимой: файл в игноре — потеря, как любой другой.
    #[test]
    fn a_replacement_that_does_not_regenerate_the_version_file_protects_it() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::write(root.join(".gitignore"), format!("{VERSION_FILE_NAME}\n")).expect("ignore");

        let asked = root.join("cf");
        fs::create_dir_all(&asked).expect("cf");
        fs::write(asked.join(VERSION_FILE_NAME), "<info/>\n").expect("version file");
        assert!(matches!(
            guard_replacement(&cli(), &asked, &ask_first(), &[], Destruction::Replace),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn the_refusal_is_one_line() {
        let message = cli_refusal(
            Path::new("/project/src/cf"),
            &[PathBuf::from("a.xml"), PathBuf::from("b.xml")],
        );
        assert_eq!(message.lines().count(), 1, "{message}");
    }

    #[test]
    fn a_long_list_is_cut_and_counted() {
        let paths: Vec<PathBuf> = (0..NAMED_LOSS_LIMIT + 5)
            .map(|i| PathBuf::from(format!("src/cf/file{i}.xml")))
            .collect();
        let message = cli_refusal(Path::new("/project/src/cf"), &paths);
        assert!(message.contains("and 5 more"), "{message}");
        assert!(message.contains("file19.xml"), "{message}");
        assert!(!message.contains("file20.xml"), "{message}");
    }
}
