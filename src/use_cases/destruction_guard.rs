//! Сторож: не дать замене каталога уничтожить работу, которую не вернуть.
//!
//! Платформа не знает о том, что человек держит в каталоге исходников, а замена
//! каталога стирает оттуда всё лишнее безвозвратно: резервную копию прежнего
//! содержимого раннер до сих пор удалял последним шагом.
//!
//! Спрашивают об этом систему контроля версий, и ответов у неё три, а не два.
//! Незнание — законный ответ: раннер работает и там, где гита нет вовсе.
//!
//! Что делать с незнанием — решено намеренно: работа идёт, как шла до сторожа.
//! Защитить того, за кого нельзя ответить, здесь нечем, а изображать защиту
//! дороже, чем её не обещать: сохранять копию дерева на **каждой** выгрузке вне
//! репозитория значит платить за редкий случай на общем пути. Настоящий ответ для
//! таких каталогов — не гит, а собственная память раннера о том, что он сам
//! породил (#163, #165).

use std::path::{Path, PathBuf};

use crate::platform::git::{uncommitted_work_in, UncommittedWork};
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, ExecutionTransport};
use crate::use_cases::request::ConsentKey;

/// Сколько потерь перечислять в отказе, прежде чем считать их числом.
const NAMED_LOSS_LIMIT: usize = 20;

/// Чьё содержимое лежит в каталоге и разрешено ли его уничтожить.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DestructionConsent {
    /// Каталог раннер завёл для себя: кеш инструментов и тому подобное. Спрашивать
    /// систему контроля версий не о чем.
    RunnerOwned,
    /// Каталог назвал человек. Незафиксированное останавливает работу, и отказ называет
    /// выходы, которые у вызывающего есть.
    AskFirst(WaysOut),
    /// Человек попросил уничтожить явно.
    Granted,
}

impl DestructionConsent {
    /// Согласие по просьбе вызывающего: уничтожить, если он попросил, иначе спросить.
    pub(super) fn requested(discard_uncommitted: bool, ways_out: WaysOut) -> Self {
        if discard_uncommitted {
            Self::Granted
        } else {
            Self::AskFirst(ways_out)
        }
    }
}

/// Выходы из отказа. Их сообщает вызывающий: только он знает, какой ключ согласия есть у
/// его команды и какой командой строки та же цель достижима.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WaysOut {
    /// Ключ согласия команды вызывающего.
    pub(super) key: ConsentKey,
    /// Та же цель командой строки без ключа — например, `pull main`. Её называет отказ
    /// транспорту, у которого ключа нет (MCP). `None` — точно собрать нельзя.
    pub(super) cli_command: Option<String>,
}

/// Отказывает до того, как что-либо стёрто, либо пропускает работу дальше.
///
/// `regenerated` — имена файлов в корне `target`, которые эта замена пишет заново;
/// их называет вызывающий, потому что только он знает, что пишет. Выгрузка в
/// формате Конфигуратора передаёт опись версий: платформа пишет её в каждую полную
/// выгрузку, а штатно опись лежит в игноре, и без исключения отказ стоял бы на
/// каждой выгрузке. Преобразование и замена проекта EDT описи не пишут — у них
/// исключений нет.
///
/// `context` называет транспорт: повторить вызов человек командной строки и клиент MCP
/// могут по-разному.
pub(super) fn guard_replacement(
    context: &ExecutionContext,
    target: &Path,
    consent: &DestructionConsent,
    regenerated: &[&str],
) -> Result<(), AppError> {
    let ways_out = match consent {
        DestructionConsent::RunnerOwned => return Ok(()),
        DestructionConsent::Granted => None,
        DestructionConsent::AskFirst(ways_out) => Some(ways_out),
    };

    match uncommitted_work_in(target, regenerated) {
        // Терять нечего: прежнее содержимое система контроля версий вернёт сама.
        UncommittedWork::Nothing => Ok(()),
        UncommittedWork::AtRisk(paths) => match ways_out {
            // Попросили уничтожить — уничтожаем, как и обещает имя ключа.
            None => Ok(()),
            Some(ways_out) => Err(AppError::Validation(refusal(
                target,
                &paths,
                ways_out,
                context.transport(),
            ))),
        },
        // Ответа нет — работа идёт, как шла до сторожа. Это не защита и не
        // выдаётся за неё.
        UncommittedWork::Unknown(_) => Ok(()),
    }
}

fn refusal(
    target: &Path,
    paths: &[PathBuf],
    ways_out: &WaysOut,
    transport: ExecutionTransport,
) -> String {
    let named: Vec<String> = paths
        .iter()
        .take(NAMED_LOSS_LIMIT)
        .map(|path| path.display().to_string())
        .collect();
    let rest = paths.len().saturating_sub(named.len());
    let tail = if rest > 0 {
        format!(", and {rest} more")
    } else {
        String::new()
    };
    // Одной строкой: человеческий вывод — закреплённая форма, и многострочная
    // подробность в нём рассыпается по разным видам строк.
    format!(
        "refusing to replace '{}': {} file(s) there exist nowhere else ({}{}); {}",
        target.display(),
        paths.len(),
        named.join(", "),
        tail,
        remedy(ways_out, transport)
    )
}

/// Выход из отказа, который у вызывающего есть.
///
/// Готовой команды из имени команды отказ не собирает: урезанная до имени, она теряет
/// набор, каталог вывода и прочие аргументы, и буквальный повтор бьёт в чужой каталог.
/// В командной строке совет — тот же вызов с добавленным ключом, и только тому, у кого
/// ключ есть. У MCP ключа нет: отказ называет команду строки для той же цели, собранную
/// вызывающим.
fn remedy(ways_out: &WaysOut, transport: ExecutionTransport) -> String {
    const DISCARDS: &str = "which replaces the directory and discards them";
    match (transport, ways_out.key) {
        (ExecutionTransport::Cli, ConsentKey::Absent) => {
            "commit or stash them and run the same command again".to_owned()
        }
        (ExecutionTransport::Cli, ConsentKey::Force) => format!(
            "commit or stash them and run the same command again, or repeat the same command with `--force` added, {DISCARDS}"
        ),
        (ExecutionTransport::McpStdio | ExecutionTransport::McpHttp, ConsentKey::Absent) => {
            "commit or stash them and call the tool again".to_owned()
        }
        (ExecutionTransport::McpStdio | ExecutionTransport::McpHttp, ConsentKey::Force) => {
            let command = match &ways_out.cli_command {
                Some(command) => format!("`v8-runner {command} --force`"),
                None => {
                    "the matching `v8-runner` command with `--force` for the same target".to_owned()
                }
            };
            format!(
                "commit or stash them and call the tool again, or run {command} from the command line, {DISCARDS}"
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

    use crate::use_cases::context::CommandName;

    fn cli() -> ExecutionContext {
        ExecutionContext::cli(CommandName::Dump)
    }

    fn with_force(cli_command: Option<&str>) -> WaysOut {
        WaysOut {
            key: ConsentKey::Force,
            cli_command: cli_command.map(str::to_owned),
        }
    }

    fn ask_first() -> DestructionConsent {
        DestructionConsent::AskFirst(with_force(Some("pull main")))
    }

    fn cli_refusal(target: &Path, paths: &[PathBuf]) -> String {
        refusal(target, paths, &with_force(None), ExecutionTransport::Cli)
    }

    #[test]
    fn a_runner_owned_directory_is_never_questioned() {
        let dir = tempdir().expect("tempdir");
        assert!(
            guard_replacement(&cli(), dir.path(), &DestructionConsent::RunnerOwned, &[]).is_ok()
        );
    }

    /// Вне репозитория ответа нет — и сторож не притворяется, что защитил.
    #[test]
    fn without_an_answer_the_work_goes_on_as_before() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("hand-written.xml"), "mine\n").expect("write");
        assert!(guard_replacement(&cli(), dir.path(), &ask_first(), &[]).is_ok());
    }

    #[test]
    fn the_refusal_names_what_would_be_lost() {
        let message = cli_refusal(
            Path::new("/project/src/cf"),
            &[PathBuf::from("src/cf/hand-written.xml")],
        );
        assert!(message.contains("src/cf/hand-written.xml"), "{message}");
    }

    /// Отказ называет выходы: сохранить работу и повторить либо заменить каталог с её
    /// потерей — и только тем путём, который у вызывающего действительно есть. Готовой
    /// команды из имени команды он не собирает: урезанная, она бьёт в другой каталог.
    #[test]
    fn the_refusal_names_the_ways_out_the_caller_has() {
        let lost = [PathBuf::from("src/cf/hand-written.xml")];
        let target = Path::new("/project/src/cf");
        let without_key = WaysOut {
            key: ConsentKey::Absent,
            cli_command: None,
        };

        let cli = refusal(
            target,
            &lost,
            &with_force(Some("pull ext")),
            ExecutionTransport::Cli,
        );
        assert!(
            cli.contains("commit or stash them and run the same command again"),
            "{cli}"
        );
        assert!(
            cli.contains("repeat the same command with `--force` added"),
            "{cli}"
        );
        assert!(!cli.contains("`pull --force`"), "{cli}");
        assert!(!cli.contains("`pull ext"), "{cli}");

        let no_key = refusal(target, &lost, &without_key, ExecutionTransport::Cli);
        assert!(
            no_key.contains("commit or stash them and run the same command again"),
            "{no_key}"
        );
        assert!(!no_key.contains("--force"), "{no_key}");

        for transport in [ExecutionTransport::McpStdio, ExecutionTransport::McpHttp] {
            let mcp = refusal(target, &lost, &with_force(Some("pull ext")), transport);
            assert!(
                mcp.contains("commit or stash them and call the tool again"),
                "{mcp}"
            );
            assert!(
                mcp.contains("run `v8-runner pull ext --force` from the command line"),
                "{mcp}"
            );
            assert!(!mcp.contains("pass --force"), "{mcp}");

            let unknown = refusal(target, &lost, &with_force(None), transport);
            assert!(
                unknown.contains("the matching `v8-runner` command with `--force`"),
                "{unknown}"
            );

            let no_key = refusal(target, &lost, &without_key, transport);
            assert!(!no_key.contains("--force"), "{no_key}");
        }
    }

    /// Попросили явно — уничтожаем, как и обещает имя ключа.
    #[test]
    fn an_explicit_request_discards_instead_of_hoarding() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert!(guard_replacement(&cli(), root, &DestructionConsent::Granted, &[]).is_ok());
        assert!(matches!(
            guard_replacement(&cli(), root, &ask_first(), &[]),
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
        assert!(guard_replacement(&cli(), &asked, &ask_first(), &[VERSION_FILE_NAME]).is_ok());
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
            guard_replacement(&cli(), &asked, &ask_first(), &[]),
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
