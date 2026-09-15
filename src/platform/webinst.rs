//! Состав команды `webinst` по грамматике платформы (Приложение 4, 4.14).
//!
//! `webinst [-publish] | -delete <-iis | -apache2 | -apache22 | -apache24> -wsdir <каталог>
//! -dir <каталог> -connstr <строка> [-confpath <файл>] [-descriptor <файл>] [-osauth]`.
//! Публикация замещает `default.vrd` целиком; удаление не требует строки соединения.

use std::path::Path;

use crate::config::model::{InfobaseWebConfig, WebServerKind};
use crate::domain::publish::PublishAction;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_publication_names_the_server_directories_and_connection() {
        let web = InfobaseWebConfig {
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
