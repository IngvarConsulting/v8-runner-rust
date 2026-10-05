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
use crate::use_cases::context::{shell_word, ExecutionContext, ExecutionTransport};

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

/// Выходы из отказа, кроме общего для всех «сохранить работу и повторить». Их называет
/// вызывающий: только он знает, есть ли у его цели замена в командной строке и какая.
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

/// Отказывает до того, как что-либо стёрто, либо пропускает работу дальше.
///
/// `regenerated` — имена файлов в корне `target`, которые эта замена пишет заново;
/// их называет вызывающий, потому что только он знает, что пишет. Выгрузка в
/// формате Конфигуратора передаёт опись версий: платформа пишет её в каждую полную
/// выгрузку, а штатно опись лежит в игноре, и без исключения отказ стоял бы на
/// каждой выгрузке. Преобразование и замена проекта EDT описи не пишут — у них
/// исключений нет.
///
/// `context` называет транспорт и глобальные ключи запуска: повторить вызов человек
/// командной строки и клиент MCP могут по-разному, а команда в совете должна попасть в ту
/// же цель.
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
                target, &paths, ways_out, context,
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
    context: &ExecutionContext,
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
        remedy(ways_out, context)
    )
}

/// Выход из отказа, который у вызывающего есть.
///
/// Готовой команды, урезанной до имени команды, отказ не собирает: без набора, каталога
/// вывода и глобальных ключей буквальный повтор бьёт в чужой каталог или чужую базу.
/// Точная команда `pull` несёт набор и глобальные ключи запуска; у MCP по HTTP она
/// исполнима только там, где работает сервер.
fn remedy(ways_out: &WaysOut, context: &ExecutionContext) -> String {
    const DISCARDS: &str = "which replaces the directory and discards them";
    let transport = context.transport();
    let save = match transport {
        ExecutionTransport::Cli => "commit or stash them and run the same command again",
        ExecutionTransport::McpStdio | ExecutionTransport::McpHttp => {
            "commit or stash them and call the tool again"
        }
    };
    match ways_out {
        WaysOut::SaveWork => save.to_owned(),
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
            let command =
                context.advised_command(&format!("pull {} --force", shell_word(source_set)));
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

    use crate::use_cases::context::{CommandLineTarget, CommandName};

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

    fn cli_refusal(target: &Path, paths: &[PathBuf]) -> String {
        refusal(target, paths, &WaysOut::SameCallWithForce, &cli())
    }

    /// Сервер, запущенный с конфигом в другом каталоге, с невыбранной по умолчанию базой
    /// и переопределённым рабочим каталогом.
    fn started_elsewhere() -> CommandLineTarget {
        CommandLineTarget {
            config: Some(PathBuf::from("/srv/my project/v8project.yaml")),
            infobase: Some("staging".to_owned()),
            workdir: Some(PathBuf::from("/var/tmp/v8w")),
        }
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

    /// Отказ называет выходы, которые у вызывающего есть: сохранить работу и повторить —
    /// всегда, а заменить каталог — только тому, кому это доступно.
    #[test]
    fn the_refusal_names_the_ways_out_the_caller_has() {
        let lost = [PathBuf::from("src/cf/hand-written.xml")];
        let target = Path::new("/project/src/cf");

        let same_call = refusal(target, &lost, &WaysOut::SameCallWithForce, &cli());
        assert!(
            same_call.contains("commit or stash them and run the same command again"),
            "{same_call}"
        );
        assert!(
            same_call.contains("repeat the same command with `--force` added"),
            "{same_call}"
        );

        let pull = refusal(target, &lost, &pull_force("ext"), &cli());
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
            let save_only = refusal(target, &lost, &WaysOut::SaveWork, &context);
            assert!(save_only.contains("commit or stash them"), "{save_only}");
            assert!(!save_only.contains("--force"), "{save_only}");
        }

        for context in [
            ExecutionContext::mcp_stdio(CommandName::Dump),
            ExecutionContext::mcp_http(CommandName::Dump),
        ] {
            let mcp = refusal(target, &lost, &pull_force("ext"), &context);
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
            let message = refusal(target, &lost, &pull_force("ext"), &context);
            assert!(message.contains(expected), "{transport:?}: {message}");
            assert!(
                message
                    .contains("a full dump of source-set 'ext' that replaces its whole directory"),
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
