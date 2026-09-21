/// Parsed V8 connection and optional authentication parameters.
#[derive(Debug, Clone)]
pub struct V8Connection {
    raw: String,
    connection_args: Vec<String>,
    /// Optional username added as `/N <value>`.
    pub user: Option<String>,
    /// Optional password added as `/P <value>`.
    pub password: Option<String>,
}

impl V8Connection {
    /// Build a reusable connection model from a raw connection string.
    pub fn from_connection_string(raw: &str) -> Self {
        let trimmed = raw.trim();
        let connection_args = if trimmed.starts_with('/') || trimmed.starts_with('-') {
            split_arg_string(trimmed)
        } else {
            vec!["/IBConnectionString".to_owned(), trimmed.to_owned()]
        };

        Self {
            raw: trimmed.to_owned(),
            connection_args,
            user: None,
            password: None,
        }
    }

    /// Build CLI arguments for a V8 utility launch.
    pub fn args(&self) -> Vec<String> {
        let mut args = self.connection_args.clone();
        args.extend(self.credential_args());
        args
    }

    /// Только реквизиты базы, без адреса: `/N` и `/P`.
    ///
    /// Отделены от адреса, потому что связка «адрес + реквизиты» верна не всегда.
    /// У автономной цели `infobase.user` и `infobase.password` — учётные данные
    /// SSH-шлюза, а не базы, и клиенту их отдавать нельзя.
    pub fn credential_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(user) = &self.user {
            args.push("/N".to_owned());
            args.push(user.clone());
        }
        if let Some(password) = &self.password {
            if !password.is_empty() {
                args.push("/P".to_owned());
                args.push(password.clone());
            }
        }
        args
    }

    /// Only the infobase address, without credentials: for launch modes where `/N` and
    /// `/P` are ignored and the password must not reach the command line.
    pub fn infobase_args(&self) -> Vec<String> {
        self.connection_args.clone()
    }

    /// Return the file-based infobase path when connection string contains `File=...`.
    pub fn file_path(&self) -> Option<&str> {
        if self.raw.starts_with('/') || self.raw.starts_with('-') {
            return file_path_from_args(&self.connection_args);
        }

        declared_parameters(&self.raw)?
            .into_iter()
            .find(|(key, _)| key == "file")
            .map(|(_, value)| value)
    }

    /// Returns whether the raw value has a supported file or server connection shape.
    pub fn has_supported_shape(&self) -> bool {
        if let Some(path) = self.file_path() {
            return !path.trim().is_empty();
        }
        if self.raw.starts_with('/') || self.raw.starts_with('-') {
            let mut args = self.connection_args.iter();
            while let Some(arg) = args.next() {
                if arg.eq_ignore_ascii_case("/s") || arg.eq_ignore_ascii_case("-s") {
                    return args.next().is_some_and(|value| {
                        let value = value.trim();
                        !value.is_empty() && value.contains('\\')
                    });
                }
            }
            return false;
        }

        // Платформа принимает завершающую `;` и значения в кавычках: `Srvr="srv";Ref="ut";`
        // — такая же серверная строка, как без них.
        let Some(parameters) = declared_parameters(&self.raw) else {
            return false;
        };
        let mut server = None;
        let mut reference = None;
        for (key, value) in parameters {
            let value = unquote_connection_value(value);
            if ['"', '\'']
                .iter()
                .any(|quote| value.starts_with(*quote) || value.ends_with(*quote))
            {
                // Непарная кавычка: платформа такую строку не примет.
                return false;
            }
            match key.as_str() {
                "srvr" => server = Some(value),
                "ref" => reference = Some(value),
                _ => {}
            }
        }
        server.is_some_and(|value| !value.trim().is_empty())
            && reference.is_some_and(|value| !value.trim().is_empty())
    }

    /// Returns a stable file-based infobase connection string when available.
    pub fn create_infobase_arg(&self) -> Option<String> {
        self.file_path()
            .map(|path| format!("File='{}'", path.replace('\'', "''")))
    }
}

/// Параметры объявленной формы строки подключения — `ключ=значение` через `;`: ключ
/// строчными и без пробелов вокруг, значение без пробелов по краям, пустые части
/// (завершающая `;`) пропущены. `None` — часть без `=`: такую строку платформа не разберёт.
/// Один разбор на все вопросы к строке, чтобы `File = …` не читался одним местом как
/// файловый адрес, а другим — как серверный.
pub fn declared_parameters(raw: &str) -> Option<Vec<(String, &str)>> {
    raw.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once('=')
                .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim()))
        })
        .collect()
}

/// Значение параметра строки подключения без обрамляющих кавычек: платформа принимает
/// `Srvr="srv"` и `Srvr='srv'` наравне с `Srvr=srv`.
pub fn unquote_connection_value(value: &str) -> &str {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(value);
    };
    inner
}

fn file_path_from_args(args: &[String]) -> Option<&str> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.eq_ignore_ascii_case("/f") || arg.eq_ignore_ascii_case("-f") {
            return args.next().map(String::as_str);
        }
    }

    None
}

fn split_arg_string(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in raw.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::V8Connection;

    #[test]
    fn wraps_plain_connection_string_as_flag_and_value() {
        let connection = V8Connection::from_connection_string("File=/tmp/ib");

        assert_eq!(
            connection.args(),
            vec!["/IBConnectionString", "File=/tmp/ib"]
        );
    }

    #[test]
    fn splits_raw_connection_and_auth_into_separate_tokens() {
        let mut connection = V8Connection::from_connection_string("/F \"/tmp/my ib\"");
        connection.user = Some("alice".to_owned());
        connection.password = Some("secret".to_owned());

        assert_eq!(
            connection.args(),
            vec!["/F", "/tmp/my ib", "/N", "alice", "/P", "secret"]
        );
    }

    #[test]
    fn extracts_file_path_from_connection_string() {
        let connection = V8Connection::from_connection_string("Srvr=demo;File=/tmp/ib;Ref=test");

        assert_eq!(connection.file_path(), Some("/tmp/ib"));
    }

    #[test]
    fn validates_supported_file_and_server_connection_shapes() {
        assert!(V8Connection::from_connection_string("File=/tmp/ib").has_supported_shape());
        assert!(
            V8Connection::from_connection_string("Srvr=cluster;Ref=demo").has_supported_shape()
        );
        assert!(V8Connection::from_connection_string(r#"/S "cluster\demo""#).has_supported_shape());
        assert!(!V8Connection::from_connection_string("not a connection").has_supported_shape());
        assert!(!V8Connection::from_connection_string("Srvr=cluster;Ref=").has_supported_shape());
        assert!(!V8Connection::from_connection_string("File=").has_supported_shape());
    }

    /// Ключ `File` с пробелами вокруг `=` читается тем же разбором, что и `Srvr`/`Ref`:
    /// иначе одна строка была бы файловой для одного вопроса и серверной для другого.
    #[test]
    fn a_spaced_file_key_is_still_a_file_address() {
        for raw in ["File = /srv/ib", "file=/srv/ib", " FILE =/srv/ib ;"] {
            let connection = V8Connection::from_connection_string(raw);
            assert_eq!(connection.file_path(), Some("/srv/ib"), "{raw}");
            assert!(connection.has_supported_shape(), "{raw}");
        }
        assert_eq!(
            V8Connection::from_connection_string("File = /srv/ib;Srvr=srv;Ref=db").file_path(),
            Some("/srv/ib"),
            "a file address wins over server parts in the same string"
        );
    }

    /// Канонические формы платформы: завершающая `;`, кавычки, пробелы вокруг `=` и `;`,
    /// дополнительные параметры.
    #[test]
    fn a_server_connection_keeps_its_shape_with_a_trailing_separator_quotes_and_spaces() {
        for raw in [
            "Srvr=host;Ref=name;",
            "Srvr=\"host:1541\";Ref=\"name\";",
            "Srvr='host';Ref='name'",
            " Srvr = host ; Ref = name ",
            "Srvr=host;Ref=name;Usr=a;Pwd=b",
        ] {
            assert!(
                V8Connection::from_connection_string(raw).has_supported_shape(),
                "{raw}"
            );
        }
        for raw in [
            "Srvr=\"\";Ref=name",
            "Srvr=host;Ref=;",
            "Srvr=host",
            ";",
            "Srvr=\"host;Ref=x",
            "Srvr=host;Ref='x",
        ] {
            assert!(
                !V8Connection::from_connection_string(raw).has_supported_shape(),
                "{raw}"
            );
        }
    }

    #[test]
    fn extracts_file_path_from_raw_f_args() {
        let connection = V8Connection::from_connection_string("/F \"/tmp/my ib\"");

        assert_eq!(connection.file_path(), Some("/tmp/my ib"));
    }

    #[test]
    fn extracts_file_path_from_dash_f_args() {
        let connection = V8Connection::from_connection_string("-F /tmp/ib");

        assert_eq!(connection.file_path(), Some("/tmp/ib"));
    }

    #[test]
    fn trims_leading_whitespace_before_parsing_raw_args() {
        let connection = V8Connection::from_connection_string("  /F /tmp/ib  ");

        assert_eq!(connection.args(), vec!["/F", "/tmp/ib"]);
        assert_eq!(connection.file_path(), Some("/tmp/ib"));
    }
}
