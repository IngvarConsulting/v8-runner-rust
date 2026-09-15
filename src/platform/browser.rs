//! Открытие адреса в браузере пользователя.
//!
//! Веб-клиент — это адрес, а не утилита платформы: раннер только просит операционную
//! систему открыть его тем, чем она открывает ссылки. Доступность адреса не проверяется.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::platform::process::ProcessError;

/// Программа, которой система открывает ссылки, и её аргументы перед адресом.
pub fn opener() -> (PathBuf, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        (PathBuf::from("/usr/bin/open"), Vec::new())
    }
    #[cfg(target_os = "windows")]
    {
        (
            PathBuf::from("cmd"),
            vec!["/C".to_owned(), "start".to_owned(), String::new()],
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        (PathBuf::from("xdg-open"), Vec::new())
    }
}

/// Просит систему открыть адрес и не ждёт браузера.
pub fn open_url(program: &Path, leading_args: &[String], url: &str) -> Result<u32, ProcessError> {
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
    Ok(child.id())
}
