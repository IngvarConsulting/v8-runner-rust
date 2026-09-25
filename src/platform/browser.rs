//! Открытие адреса в браузере пользователя.
//!
//! Веб-клиент — это адрес, а не утилита платформы: раннер только просит операционную
//! систему открыть его тем, чем она открывает ссылки. Доступность адреса не проверяется.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::platform::process::{ProcessError, WorkGiven};

/// Программа, которой система открывает ссылки, и её аргументы перед адресом.
pub fn opener() -> (PathBuf, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        (PathBuf::from("/usr/bin/open"), Vec::new())
    }
    #[cfg(target_os = "windows")]
    {
        // Не `cmd /C start`: интерпретатор разбирает `&`, `|`, `^`, `<`, `>` сам, а
        // стандартное экранирование Rust делается под `CommandLineToArgvW`, которым
        // `cmd.exe` не пользуется. Адрес приходит из `infobase.web.url`, где `&`
        // совершенно законен (`?N=user&W=1`) и пробелов не содержит, поэтому дошёл бы
        // до `cmd` без кавычек и разделил бы команду. `FileProtocolHandler` открывает
        // ссылку тем же обработчиком, но ничего не переразбирает.
        (
            PathBuf::from("rundll32.exe"),
            vec!["url.dll,FileProtocolHandler".to_owned()],
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        (PathBuf::from("xdg-open"), Vec::new())
    }
}

/// Адрес, пригодный для передачи системному обработчику ссылок.
///
/// Управляющие символы и кавычки отвергаются до запуска: они не встречаются в законном
/// адресе и существуют в нём только затем, чтобы что-нибудь разделить.
fn ensure_openable(url: &str) -> Result<(), ProcessError> {
    if url.is_empty() || url.chars().any(|ch| ch.is_control() || ch == '"') {
        return Err(ProcessError::SpawnFailed {
            cmd: "open url".to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("address is not openable: {url:?}"),
            ),
        });
    }
    Ok(())
}

/// Просит систему открыть адрес и не ждёт браузера. Запущенная программа открытия — работа
/// команды: отметка ставится, как только она запущена.
pub fn open_url(
    program: &Path,
    leading_args: &[String],
    url: &str,
    work: &WorkGiven,
) -> Result<u32, ProcessError> {
    ensure_openable(url)?;
    let child = Command::new(program)
        .args(leading_args)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| ProcessError::SpawnFailed {
            cmd: program.display().to_string(),
            source,
        })?;
    work.mark_work_given();
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Адрес идёт системному обработчику ссылок как аргумент, и управляющие символы в
    /// нём не нужны никому, кроме того, кто хочет разделить команду.
    #[test]
    fn an_address_with_control_characters_is_refused_before_launch() {
        ensure_openable("http://host/base/?N=user&W=1").expect("an ordinary web address");
        ensure_openable("http://host/base/").expect("a plain address");

        for hostile in [
            "http://host/\r\nnet user",
            "http://host/\0",
            "http://host/\"x",
            "",
        ] {
            ensure_openable(hostile).expect_err(&format!("must refuse {hostile:?}"));
        }
    }

    /// Запущенная программа открытия — работа команды. Адрес, отвергнутый до запуска, и
    /// программа, которой нет, работы не дают.
    #[cfg(unix)]
    #[test]
    fn only_a_started_opener_marks_the_work() {
        let shell = Path::new("/bin/sh");
        let quiet = ["-c".to_owned(), "exit 0".to_owned()];

        let refused = WorkGiven::for_command();
        open_url(shell, &quiet, "http://host/\r\nx", &refused).expect_err("a hostile address");
        assert!(
            !refused.given(),
            "an address refused before the start gives no work"
        );

        let missing = WorkGiven::for_command();
        open_url(
            Path::new("/nonexistent/opener"),
            &[],
            "http://host/",
            &missing,
        )
        .expect_err("no opener");
        assert!(
            !missing.given(),
            "an opener that could not start gives no work"
        );

        let opened = WorkGiven::for_command();
        open_url(shell, &quiet, "http://host/", &opened).expect("the opener started");
        assert!(opened.given(), "a started opener is the command's work");
    }

    /// На Windows ссылку открывает обработчик протокола, а не интерпретатор команд:
    /// `cmd` переразобрал бы `&` в адресе и разделил бы команду.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_opens_links_without_a_command_interpreter() {
        let (program, _) = opener();
        assert!(
            !program.to_string_lossy().eq_ignore_ascii_case("cmd"),
            "the command interpreter re-parses metacharacters in the address"
        );
    }
}
