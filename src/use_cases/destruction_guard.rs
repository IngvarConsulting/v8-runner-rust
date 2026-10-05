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
use crate::use_cases::context::{CommandName, ExecutionContext, ExecutionTransport};

/// Сколько потерь перечислять в отказе, прежде чем считать их числом.
const NAMED_LOSS_LIMIT: usize = 20;

/// Чьё содержимое лежит в каталоге и разрешено ли его уничтожить.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DestructionConsent {
    /// Каталог раннер завёл для себя: кеш инструментов и тому подобное. Спрашивать
    /// систему контроля версий не о чем.
    RunnerOwned,
    /// Каталог назвал человек. Незафиксированное останавливает работу.
    AskFirst,
    /// Человек попросил уничтожить явно.
    Granted,
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
/// `context` называет команду и транспорт: отказ советует путь, который у вызывающего есть.
pub(super) fn guard_replacement(
    context: &ExecutionContext,
    target: &Path,
    consent: DestructionConsent,
    regenerated: &[&str],
) -> Result<(), AppError> {
    if consent == DestructionConsent::RunnerOwned {
        return Ok(());
    }

    match uncommitted_work_in(target, regenerated) {
        // Терять нечего: прежнее содержимое система контроля версий вернёт сама.
        UncommittedWork::Nothing => Ok(()),
        // Попросили уничтожить — уничтожаем, как и обещает имя ключа.
        UncommittedWork::AtRisk(_) if consent == DestructionConsent::Granted => Ok(()),
        UncommittedWork::AtRisk(paths) => Err(AppError::Validation(refusal(
            target,
            &paths,
            context.command(),
            context.transport(),
        ))),
        // Ответа нет — работа идёт, как шла до сторожа. Это не защита и не
        // выдаётся за неё.
        UncommittedWork::Unknown(_) => Ok(()),
    }
}

fn refusal(
    target: &Path,
    paths: &[PathBuf],
    command: CommandName,
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
        remedy(command, transport)
    )
}

/// Выход из отказа, который у вызывающего есть. Согласие на уничтожение даёт только ключ
/// командной строки: у MCP его нет, и инструмент называет команду, а не ключ.
fn remedy(command: CommandName, transport: ExecutionTransport) -> String {
    let command = command.as_str();
    match transport {
        ExecutionTransport::Cli => format!(
            "commit or stash them and run `{command}` again, or run `{command} --force` to replace the directory and discard them"
        ),
        ExecutionTransport::McpStdio | ExecutionTransport::McpHttp => format!(
            "commit or stash them and call the tool again, or run `v8-runner {command} --force` from the command line to replace the directory and discard them"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_git::init_git_repo;
    use crate::use_cases::ignored_files::VERSION_FILE_NAME;
    use std::fs;
    use tempfile::tempdir;

    fn cli() -> ExecutionContext {
        ExecutionContext::cli(CommandName::Dump)
    }

    fn cli_refusal(target: &Path, paths: &[PathBuf]) -> String {
        refusal(target, paths, CommandName::Dump, ExecutionTransport::Cli)
    }

    #[test]
    fn a_runner_owned_directory_is_never_questioned() {
        let dir = tempdir().expect("tempdir");
        assert!(
            guard_replacement(&cli(), dir.path(), DestructionConsent::RunnerOwned, &[]).is_ok()
        );
    }

    /// Вне репозитория ответа нет — и сторож не притворяется, что защитил.
    #[test]
    fn without_an_answer_the_work_goes_on_as_before() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("hand-written.xml"), "mine\n").expect("write");
        assert!(guard_replacement(&cli(), dir.path(), DestructionConsent::AskFirst, &[]).is_ok());
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
    /// потерей — тем путём, который у вызывающего действительно есть.
    #[test]
    fn the_refusal_names_the_ways_out_the_caller_has() {
        let lost = [PathBuf::from("src/cf/hand-written.xml")];
        let target = Path::new("/project/src/cf");

        let cli = refusal(target, &lost, CommandName::Dump, ExecutionTransport::Cli);
        assert!(
            cli.contains("commit or stash them and run `pull` again"),
            "{cli}"
        );
        assert!(
            cli.contains("run `pull --force` to replace the directory and discard them"),
            "{cli}"
        );

        let convert = refusal(target, &lost, CommandName::Convert, ExecutionTransport::Cli);
        assert!(convert.contains("run `convert --force`"), "{convert}");

        for transport in [ExecutionTransport::McpStdio, ExecutionTransport::McpHttp] {
            let mcp = refusal(target, &lost, CommandName::Dump, transport);
            assert!(
                mcp.contains("commit or stash them and call the tool again"),
                "{mcp}"
            );
            assert!(
                mcp.contains("run `v8-runner pull --force` from the command line"),
                "{mcp}"
            );
            assert!(!mcp.contains("pass --force"), "{mcp}");
        }
    }

    /// Попросили явно — уничтожаем, как и обещает имя ключа.
    #[test]
    fn an_explicit_request_discards_instead_of_hoarding() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        init_git_repo(root);
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert!(guard_replacement(&cli(), root, DestructionConsent::Granted, &[]).is_ok());
        assert!(matches!(
            guard_replacement(&cli(), root, DestructionConsent::AskFirst, &[]),
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
            DestructionConsent::AskFirst,
            &[VERSION_FILE_NAME]
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
            guard_replacement(&cli(), &asked, DestructionConsent::AskFirst, &[]),
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
