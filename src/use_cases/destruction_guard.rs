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
pub(super) fn guard_replacement(
    target: &Path,
    consent: DestructionConsent,
) -> Result<(), AppError> {
    if consent == DestructionConsent::RunnerOwned {
        return Ok(());
    }

    match uncommitted_work_in(target) {
        // Терять нечего: прежнее содержимое система контроля версий вернёт сама.
        UncommittedWork::Nothing => Ok(()),
        // Попросили уничтожить — уничтожаем, как и обещает имя ключа.
        UncommittedWork::AtRisk(_) if consent == DestructionConsent::Granted => Ok(()),
        UncommittedWork::AtRisk(paths) => Err(AppError::Validation(refusal(target, &paths))),
        // Ответа нет — работа идёт, как шла до сторожа. Это не защита и не
        // выдаётся за неё.
        UncommittedWork::Unknown(_) => Ok(()),
    }
}

fn refusal(target: &Path, paths: &[PathBuf]) -> String {
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
        "refusing to replace '{}': {} file(s) there exist nowhere else ({}{}); commit or stash them, or pass --discard-uncommitted to replace the directory anyway",
        target.display(),
        paths.len(),
        named.join(", "),
        tail
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn a_runner_owned_directory_is_never_questioned() {
        let dir = tempdir().expect("tempdir");
        assert!(guard_replacement(dir.path(), DestructionConsent::RunnerOwned).is_ok());
    }

    /// Вне репозитория ответа нет — и сторож не притворяется, что защитил.
    #[test]
    fn without_an_answer_the_work_goes_on_as_before() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("hand-written.xml"), "mine\n").expect("write");
        assert!(guard_replacement(dir.path(), DestructionConsent::AskFirst).is_ok());
    }

    #[test]
    fn the_refusal_names_what_would_be_lost() {
        let message = refusal(
            Path::new("/project/src/cf"),
            &[PathBuf::from("src/cf/hand-written.xml")],
        );
        assert!(message.contains("src/cf/hand-written.xml"), "{message}");
        assert!(message.contains("--discard-uncommitted"), "{message}");
    }

    /// Попросили явно — уничтожаем, как и обещает имя ключа.
    #[test]
    fn an_explicit_request_discards_instead_of_hoarding() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(&args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("git");
            assert!(status.success());
        }
        fs::write(root.join("hand-written.xml"), "mine\n").expect("write");

        assert!(guard_replacement(root, DestructionConsent::Granted).is_ok());
        assert!(matches!(
            guard_replacement(root, DestructionConsent::AskFirst),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn the_refusal_is_one_line() {
        let message = refusal(
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
        let message = refusal(Path::new("/project/src/cf"), &paths);
        assert!(message.contains("and 5 more"), "{message}");
        assert!(message.contains("file19.xml"), "{message}");
        assert!(!message.contains("file20.xml"), "{message}");
    }
}
