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
        } else if let Some(address) =
            declared_server_address(trimmed).and_then(|address| address.sole_s_argument())
        {
            // Объявленный серверный адрес уходит платформе её же ключом `/S host\name`:
            // рядом с `/IBConnectionString` реквизиты `/N`/`/P` она не принимала
            // (Windows, 8.3.27.1936, #55), рядом с `/S` — принимает.
            vec!["/S".to_owned(), address]
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
    /// The declared form is answered by [`declared_server_address`], the same predicate
    /// that decides how the address reaches the platform.
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

        declared_server_address(&self.raw).is_some()
    }

    /// Returns a stable file-based infobase connection string when available.
    pub fn create_infobase_arg(&self) -> Option<String> {
        self.file_path()
            .map(|path| format!("File='{}'", path.replace('\'', "''")))
    }

    /// Базу и учётную запись называет без секретов: сырая строка бывает с `Pwd=`, поэтому
    /// файловая база названа путём, серверная — именем в кластере и сервером, иная форма
    /// строки — общим словом.
    pub fn describe_target(&self) -> String {
        let target = if let Some(path) = self.file_path() {
            file_infobase(path)
        } else if let Some(address) = declared_server_address(&self.raw) {
            format!(
                "server infobase '{}' on '{}'",
                address.reference, address.server
            )
        } else {
            "the infobase".to_owned()
        };
        name_the_account(&target, self.user.as_deref())
    }
}

/// Файловая база по пути — одно имя у всех, кто её называет.
pub(crate) fn file_infobase(path: impl std::fmt::Display) -> String {
    format!("file infobase '{path}'")
}

/// Учётная запись к уже названной цели: имя пользователя, но не пароль.
pub(crate) fn name_the_account(target: &str, user: Option<&str>) -> String {
    match user.filter(|user| !user.is_empty()) {
        Some(user) => format!("{target} as '{user}'"),
        None => format!("{target} with no configured infobase user"),
    }
}

/// Серверный адрес объявленной строки: `Srvr` и `Ref` без кавычек и число частей строки.
#[derive(Debug)]
struct DeclaredServerAddress {
    server: String,
    reference: String,
    /// Сколько частей `ключ=значение` в строке.
    parts: usize,
}

impl DeclaredServerAddress {
    /// Значение ключа `/S` — `host[:port]\name`, — когда строку можно им заменить без
    /// потерь и без догадок. Условий два, и держит их сам адрес, а не тот, кто спрашивает:
    /// в строке ровно две части (дополнительные — `Locale=`, `Usr=`, иное — ключ `/S` не
    /// несёт, и терять их нельзя), и хост не перечисляет резервные серверы через запятую
    /// (справка платформы знает у `/S` одну машину, а замера списка нет — #55).
    fn sole_s_argument(&self) -> Option<String> {
        if self.parts != 2 || self.server.contains(',') {
            return None;
        }
        Some(format!("{}\\{}", self.server, self.reference))
    }
}

/// Один предикат серверной формы на все вопросы к объявленной строке — валидации и
/// сборке argv: `Srvr` и `Ref` с непустыми значениями, регистр и порядок ключей свободны,
/// завершающая `;` и парные кавычки допустимы (`Srvr="srv";Ref="ut";`). `None` — строку
/// платформа серверным адресом не считает: части нет, значение пусто или кавычка непарная.
fn declared_server_address(raw: &str) -> Option<DeclaredServerAddress> {
    let parameters = declared_parameters(raw)?;
    let mut server = None;
    let mut reference = None;
    for (key, value) in &parameters {
        let value = unquote_connection_value(value);
        if ['"', '\'']
            .iter()
            .any(|quote| value.starts_with(*quote) || value.ends_with(*quote))
        {
            // Непарная кавычка: платформа такую строку не примет.
            return None;
        }
        match key.as_str() {
            "srvr" => server = Some(value.trim()),
            "ref" => reference = Some(value.trim()),
            _ => {}
        }
    }
    let server = server.filter(|value| !value.is_empty())?;
    let reference = reference.filter(|value| !value.is_empty())?;
    Some(DeclaredServerAddress {
        server: server.to_owned(),
        reference: reference.to_owned(),
        parts: parameters.len(),
    })
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

    /// Объявленный серверный адрес уходит платформе её ключом `/S host\name`, реквизиты —
    /// отдельными `/N`/`/P`: рядом с `/IBConnectionString` платформа 8.3.27.1936 на Windows
    /// отвечала «Пользователь ИБ не идентифицирован», рядом с `/S` — подключалась (#55).
    #[test]
    fn a_declared_server_address_is_passed_as_s_with_separate_credentials() {
        let mut connection = V8Connection::from_connection_string("Srvr=\"srv\";Ref=\"ut\";");
        connection.user = Some("alice".to_owned());
        connection.password = Some("secret".to_owned());
        assert_eq!(
            connection.args(),
            vec!["/S", "srv\\ut", "/N", "alice", "/P", "secret"]
        );
        assert_eq!(connection.infobase_args(), vec!["/S", "srv\\ut"]);
        assert!(connection.has_supported_shape());

        for (raw, expected) in [
            ("Srvr=srv:1541;Ref=demo", "srv:1541\\demo"),
            (" Ref = demo ; SRVR = srv ", "srv\\demo"),
            ("Srvr=srv;Ref=demo;", "srv\\demo"),
        ] {
            assert_eq!(
                V8Connection::from_connection_string(raw).args(),
                vec!["/S", expected],
                "{raw}"
            );
        }
    }

    /// Строка с дополнительными частями, список резервных серверов, файловая строка и
    /// строка, которую платформа серверной не считает, отдаются целиком: частей терять
    /// нельзя, а форму `/S` раннер берёт только там, где она замерена — одна машина и
    /// ничего кроме адреса. Список серверов рядом с `/S` не замерен (#55), поэтому такая
    /// строка остаётся прежней формой и продолжает работать как работала.
    #[test]
    fn other_declared_strings_stay_whole_in_ibconnectionstring() {
        for raw in [
            "Srvr=host;Ref=name;Usr=a;Pwd=b",
            "Srvr=srv;Ref=demo;Locale=ru",
            "Srvr='srv1,srv2:1641';Ref=demo",
            "File=/tmp/ib;Locale=ru",
            "File=/tmp/ib",
            "Srvr=\"host;Ref=x",
            "Srvr=host",
        ] {
            assert_eq!(
                V8Connection::from_connection_string(raw).args(),
                vec!["/IBConnectionString", raw],
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

    /// Цель называется без секретов: путь файловой базы, имя в кластере и сервер —
    /// серверной, общее слово — у формы, которую назвать нечем; пароль не попадает никуда.
    #[test]
    fn a_target_is_named_without_its_secrets() {
        let mut file = V8Connection::from_connection_string("File=/srv/ib");
        file.user = Some("Admin".to_owned());
        assert_eq!(file.describe_target(), "file infobase '/srv/ib' as 'Admin'");

        let keyed = V8Connection::from_connection_string("/F /srv/ib");
        assert_eq!(
            keyed.describe_target(),
            "file infobase '/srv/ib' with no configured infobase user"
        );

        let mut server = V8Connection::from_connection_string(
            "Srvr=\"srv:1541\";Ref='demo';Usr=reader;Pwd=hidden-secret;",
        );
        server.user = Some("Admin".to_owned());
        server.password = Some("another-secret".to_owned());
        let named = server.describe_target();
        assert_eq!(named, "server infobase 'demo' on 'srv:1541' as 'Admin'");
        assert!(!named.contains("secret"), "{named}");

        let raw = V8Connection::from_connection_string("/S srv\\demo");
        assert_eq!(
            raw.describe_target(),
            "the infobase with no configured infobase user"
        );
    }
}
