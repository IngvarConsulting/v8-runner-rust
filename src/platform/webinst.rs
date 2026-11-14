//! Состав команды `webinst` по грамматике платформы (Приложение 4, 4.14).
//!
//! `webinst [-publish] | -delete <-iis | -apache2 | -apache22 | -apache24> -wsdir <каталог>
//! -dir <каталог> -connstr <строка> [-confpath <файл>] [-descriptor <файл>] [-osauth]`.
//! Публикация замещает `default.vrd` целиком; удаление не требует строки соединения.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::config::model::{InfobaseWebConfig, WebServerKind};
use crate::domain::publish::PublishAction;

use super::process::{ProcessError, ProcessResult};
use super::secrets::{mask_known_secrets, mask_text};

/// Аргументы `webinst` для объявленной публикации.
pub fn webinst_args(
    action: PublishAction,
    server: WebServerKind,
    web: &InfobaseWebConfig,
    wsdir: &str,
    dir: &Path,
    connection: &str,
) -> Vec<String> {
    let mut args = vec![
        match action {
            PublishAction::Publish => "-publish".to_owned(),
            PublishAction::Delete => "-delete".to_owned(),
        },
        format!("-{}", server.as_str()),
        "-wsdir".to_owned(),
        wsdir.to_owned(),
        "-dir".to_owned(),
        dir.display().to_string(),
    ];
    if action == PublishAction::Publish {
        args.push("-connstr".to_owned());
        args.push(connection.to_owned());
    }
    if let Some(conf) = web.conf.as_ref() {
        args.push("-confpath".to_owned());
        args.push(conf.display().to_string());
    }
    if web.os_auth && action == PublishAction::Publish {
        args.push("-osauth".to_owned());
    }
    args
}

/// Write both captured streams to the platform log, in stdout then stderr order.
pub(crate) fn write_diagnostics(
    path: &Path,
    result: &ProcessResult,
    secrets: &[&str],
) -> Result<(), ProcessError> {
    let stdout = mask_diagnostics(&result.stdout, secrets);
    let stderr = mask_diagnostics(&result.stderr, secrets);
    let mut log = File::create(path)
        .and_then(|mut log| {
            log.write_all(stdout.as_bytes())?;
            Ok(log)
        })
        .map_err(|source| ProcessError::StdoutLogIo {
            path: path.to_path_buf(),
            source,
        })?;

    let separator: &[u8] = if !stdout.is_empty() && !stderr.is_empty() && !stdout.ends_with('\n') {
        b"\n"
    } else {
        b""
    };
    log.write_all(separator)
        .and_then(|()| log.write_all(stderr.as_bytes()))
        .map_err(|source| ProcessError::StderrLogIo {
            path: path.to_path_buf(),
            source,
        })
}

fn mask_diagnostics(output: &str, secrets: &[&str]) -> String {
    let multiline_secrets: Vec<_> = secrets
        .iter()
        .copied()
        .filter(|secret| secret.contains(['\r', '\n']))
        .collect();
    let output = if multiline_secrets.is_empty() {
        std::borrow::Cow::Borrowed(output)
    } else {
        std::borrow::Cow::Owned(mask_known_secrets(output, &multiline_secrets))
    };

    // A trailing Pwd value can consume the rest of the input to the masker.
    // Keep each diagnostic and its original line ending independent.
    let mut masked = String::with_capacity(output.len());
    for line in output.split_inclusive('\n') {
        let (text, ending) = line.split_at(line.trim_end_matches(['\r', '\n']).len());
        masked.push_str(&mask_text(text, secrets));
        masked.push_str(ending);
    }
    masked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_publication_names_the_server_directories_and_connection() {
        let web = InfobaseWebConfig {
            managed: false,
            server: Some(WebServerKind::Apache24),
            wsdir: Some("demo".to_owned()),
            dir: Some(PathBuf::from("/var/www/demo")),
            conf: None,
            os_auth: false,
            url: None,
        };
        let args = webinst_args(
            PublishAction::Publish,
            WebServerKind::Apache24,
            &web,
            "demo",
            Path::new("/var/www/demo"),
            "File=/srv/ib",
        );
        assert_eq!(
            args,
            vec![
                "-publish",
                "-apache24",
                "-wsdir",
                "demo",
                "-dir",
                "/var/www/demo",
                "-connstr",
                "File=/srv/ib",
            ]
        );
    }

    /// Удаление не несёт строки соединения и ключа `-osauth`: утилите они не нужны.
    #[test]
    fn a_deletion_carries_no_connection_and_no_os_auth() {
        let web = InfobaseWebConfig {
            managed: false,
            server: Some(WebServerKind::Iis),
            wsdir: Some("demo".to_owned()),
            dir: Some(PathBuf::from("C:/inetpub/demo")),
            conf: Some(PathBuf::from("C:/conf/httpd.conf")),
            os_auth: true,
            url: None,
        };
        let args = webinst_args(
            PublishAction::Delete,
            WebServerKind::Iis,
            &web,
            "demo",
            Path::new("C:/inetpub/demo"),
            "Srvr=srv;Ref=demo",
        );
        assert_eq!(
            args,
            vec![
                "-delete",
                "-iis",
                "-wsdir",
                "demo",
                "-dir",
                "C:/inetpub/demo",
                "-confpath",
                "C:/conf/httpd.conf",
            ]
        );
    }
}
