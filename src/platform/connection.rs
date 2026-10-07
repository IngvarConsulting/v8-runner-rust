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

    /// Каталог файловой базы: относительный путь — от каталога проекта, путь через
    /// символическую ссылку — к тому же каталогу, что и прямой. `None` у серверной базы.
    pub fn file_infobase_dir(&self, base_path: &std::path::Path) -> Option<std::path::PathBuf> {
        use crate::support::path::{nearest_existing_canonical_path, resolve_from};
        let path = std::path::Path::new(unquote_connection_value(self.file_path()?));
        let absolute = resolve_from(base_path, path);
        Some(nearest_existing_canonical_path(&absolute).unwrap_or(absolute))
    }

    /// Stable address identity, excluding credentials and the selected executor.
    pub fn snapshot_identity(&self, base_path: &std::path::Path) -> Option<String> {
        use crate::support::path::snapshot_path_identity;
        if let Some(canonical) = self.file_infobase_dir(base_path) {
            return Some(format!(
                "file:{} ({})",
                snapshot_path_identity(&canonical),
                canonical.display()
            ));
        }
        // Cluster host and infobase names are case-insensitive for the platform.
        if let Some(address) = declared_server_address(&self.raw) {
            return Some(format!(
                "server:{}\\{}",
                address.server.to_lowercase(),
                address.reference.to_lowercase()
            ));
        }
        self.server_arg()
            .map(|server| format!("server:{}", server.to_lowercase()))
    }

    /// The value of the `/S` switch in the raw argument form.
    fn server_arg(&self) -> Option<&str> {
        self.connection_args
            .windows(2)
            .find(|pair| pair[0].eq_ignore_ascii_case("/s") || pair[0].eq_ignore_ascii_case("-s"))
            .map(|pair| pair[1].as_str())
    }

    /// The server part of a server connection, as declared: the value of `Srvr=`, or the
    /// part of `/S <server>\<base>` before the backslash. `None` for any other form.
    pub fn server_address(&self) -> Option<String> {
        if let Some(address) = declared_server_address(&self.raw) {
            return Some(address.server);
        }
        self.server_arg()
            .and_then(|server| server.split_once('\\'))
            .map(|(server, _)| server.trim().to_owned())
    }

    /// Серверы кластера из `Srvr=` или из `/S <server>\<base>` — каждый `host[:port]` без
    /// префикса протокола (`tcp://`), в порядке записи: основной, затем резервные. Пусто
    /// у файловой базы и у иной формы строки. Список читает `cluster_servers` — тот же
    /// разбор, по которому `DeclaredServerAddress::sole_s_argument` узнаёт резервные.
    pub fn cluster_hosts(&self) -> Vec<String> {
        self.server_address()
            .map(|server| cluster_servers(&server).map(str::to_owned).collect())
            .unwrap_or_default()
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

    /// Строка `CREATEINFOBASE` базы в кластере: адрес из этой строки подключения
    /// (`Srvr`, `Ref`) и реквизиты СУБД и администратора кластера из `creation`. Порядок
    /// и состав — как в замере #181 (8.5.4.1878): `Srvr;Ref;DBMS;DBSrvr;DB[;DBUID][;DBPwd];
    /// CrSQLDB=Y;Locale;SchJobDn=Y[;SUsr][;SPwd]`. `SchJobDn=Y` стоит всегда: созданная раннером
    /// база в кластере — с запретом регламентных заданий (решение владельца, #204). `None` — строка подключения не называет сервер и
    /// базу.
    pub fn create_cluster_infobase_arg(
        &self,
        creation: &ClusterInfobaseCreation<'_>,
    ) -> Option<String> {
        let (server, reference) = self.cluster_address()?;
        let mut parts = vec![
            connection_segment("Srvr", &server),
            connection_segment("Ref", &reference),
            connection_segment("DBMS", creation.dbms),
            connection_segment("DBSrvr", creation.database_server),
            connection_segment("DB", creation.database_name),
        ];
        let optional = [
            ("DBUID", creation.database_user),
            ("DBPwd", creation.database_password),
        ];
        parts.extend(
            optional
                .into_iter()
                .filter_map(|(key, value)| Some(connection_segment(key, value?))),
        );
        parts.push("CrSQLDB=Y".to_owned());
        parts.push(connection_segment("Locale", creation.locale));
        parts.push("SchJobDn=Y".to_owned());
        let administrator = [
            ("SUsr", creation.cluster_user),
            ("SPwd", creation.cluster_password),
        ];
        parts.extend(
            administrator
                .into_iter()
                .filter_map(|(key, value)| Some(connection_segment(key, value?))),
        );
        Some(parts.join(";"))
    }

    /// Сервер и имя базы в кластере: `Srvr` и `Ref` объявленной строки или части `/S
    /// <сервер>\<база>`.
    fn cluster_address(&self) -> Option<(String, String)> {
        if let Some(address) = declared_server_address(&self.raw) {
            return Some((address.server, address.reference));
        }
        let (server, reference) = self.server_arg()?.split_once('\\')?;
        let (server, reference) = (server.trim(), reference.trim());
        (!server.is_empty() && !reference.is_empty())
            .then(|| (server.to_owned(), reference.to_owned()))
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

/// Реквизиты создания базы в кластере, которых нет в строке подключения: СУБД, её
/// учётная запись, национальные настройки и администратор кластера.
#[derive(Debug, Clone, Copy)]
pub struct ClusterInfobaseCreation<'a> {
    pub dbms: &'a str,
    pub database_server: &'a str,
    pub database_name: &'a str,
    pub database_user: Option<&'a str>,
    pub database_password: Option<&'a str>,
    pub locale: &'a str,
    pub cluster_user: Option<&'a str>,
    pub cluster_password: Option<&'a str>,
}

/// Часть `ключ=значение` строки подключения. Значение с `;`, кавычкой или пробелом по
/// краям берётся в двойные кавычки, внутренняя кавычка удваивается; остальное идёт как есть.
fn connection_segment(key: &str, value: &str) -> String {
    let needs_quotes = value.contains([';', '"']) || value.trim() != value;
    if needs_quotes {
        format!("{key}=\"{}\"", value.replace('"', "\"\""))
    } else {
        format!("{key}={value}")
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
        if self.parts != 2 || cluster_servers(&self.server).nth(1).is_some() {
            return None;
        }
        Some(format!("{}\\{}", self.server, self.reference))
    }
}

/// Записи серверов в значении `Srvr=`: через запятую перечислены основной и резервные,
/// у каждого бывает префикс протокола (`tcp://srv:1541`). Пустая запись не выбрасывается:
/// `srv,` — тоже список, а не одна машина.
fn cluster_servers(server: &str) -> impl Iterator<Item = &str> {
    server.split(',').map(|entry| {
        let entry = entry.trim();
        entry.split_once("://").map_or(entry, |(_, rest)| rest)
    })
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
        .map(declared_parameter)
        .collect()
}

/// Одна часть объявленной формы: ключ строчными без пробелов, значение без пробелов
/// по краям. Загрузчик конфигурации разбирает части той же функцией.
pub fn declared_parameter(part: &str) -> Option<(String, &str)> {
    part.split_once('=')
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim()))
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

/// Токены сырой формы строки соединения (`/F "C:\my base" /N …`) — ровно те, что
/// уходят платформе: делит по пробелу вне кавычек и снимает кавычки. Один токенизатор на
/// все вопросы к сырой форме: и argv, и нормализация пути, и проверка учётных данных
/// смотрят на одни и те же токены.
pub(crate) fn split_arg_string(raw: &str) -> Vec<String> {
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
    use super::{ClusterInfobaseCreation, V8Connection};

    fn creation<'a>(
        cluster_user: Option<&'a str>,
        cluster_password: Option<&'a str>,
    ) -> ClusterInfobaseCreation<'a> {
        ClusterInfobaseCreation {
            dbms: "PostgreSQL",
            database_server: "db",
            database_name: "demo_db",
            database_user: Some("postgres"),
            database_password: Some("pg;pass\"word"),
            locale: "ru",
            cluster_user,
            cluster_password,
        }
    }

    /// Строка `CREATEINFOBASE` кластера — порядок и состав замера #181; значение с `;` или
    /// кавычкой берётся в кавычки, внутренняя кавычка удваивается.
    #[test]
    fn a_cluster_creation_string_follows_the_measured_form() {
        let connection = V8Connection::from_connection_string("Srvr=srv:1541;Ref=demo;");

        assert_eq!(
            connection
                .create_cluster_infobase_arg(&creation(Some("cadm"), Some("c")))
                .as_deref(),
            Some("Srvr=srv:1541;Ref=demo;DBMS=PostgreSQL;DBSrvr=db;DB=demo_db;DBUID=postgres;DBPwd=\"pg;pass\"\"word\";CrSQLDB=Y;Locale=ru;SchJobDn=Y;SUsr=cadm;SPwd=c")
        );
        assert_eq!(
            connection
                .create_cluster_infobase_arg(&creation(None, None))
                .as_deref(),
            Some("Srvr=srv:1541;Ref=demo;DBMS=PostgreSQL;DBSrvr=db;DB=demo_db;DBUID=postgres;DBPwd=\"pg;pass\"\"word\";CrSQLDB=Y;Locale=ru;SchJobDn=Y")
        );
    }

    /// Адрес берётся и из ключа `/S`, а у файловой базы строки кластера нет.
    #[test]
    fn a_cluster_creation_string_reads_the_s_form_and_refuses_a_file_base() {
        let s_form = V8Connection::from_connection_string("/S srv\\demo");
        assert!(s_form
            .create_cluster_infobase_arg(&creation(None, None))
            .is_some_and(|arg| arg.starts_with("Srvr=srv;Ref=demo;DBMS=")));
        let file = V8Connection::from_connection_string("File=/tmp/ib");
        assert_eq!(
            file.create_cluster_infobase_arg(&creation(None, None)),
            None
        );
    }

    #[test]
    fn snapshot_address_identity_ignores_credentials_and_canonicalizes_file_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("db")).expect("db");
        let declared = V8Connection::from_connection_string("File='db';Usr=a;Pwd=secret");
        let absolute = V8Connection::from_connection_string(&format!(
            "/F \"{}\"",
            dir.path().join("db").display()
        ));
        assert_eq!(
            declared.snapshot_identity(dir.path()),
            absolute.snapshot_identity(dir.path())
        );
        let server = V8Connection::from_connection_string("Srvr='host';Ref='db';Pwd=secret");
        let args = V8Connection::from_connection_string(r"/S host\db /N alice /P other");
        assert_eq!(
            server.snapshot_identity(dir.path()),
            args.snapshot_identity(dir.path())
        );
        let spelled = V8Connection::from_connection_string("srvr=HOST;ref=DB");
        let flagged = V8Connection::from_connection_string(r"/S Host\Db");
        assert_eq!(
            V8Connection::from_connection_string("Srvr=host;Ref=БАЗА")
                .snapshot_identity(dir.path()),
            V8Connection::from_connection_string(r"/S host\база").snapshot_identity(dir.path()),
            "Cyrillic infobase names are case-insensitive too"
        );
        assert_eq!(
            spelled.snapshot_identity(dir.path()),
            server.snapshot_identity(dir.path()),
            "server and infobase names are case-insensitive"
        );
        assert_eq!(
            flagged.snapshot_identity(dir.path()),
            server.snapshot_identity(dir.path())
        );
        for connection in [&declared, &server, &args] {
            let identity = connection.snapshot_identity(dir.path()).expect("identity");
            for secret in ["secret", "alice", "other"] {
                assert!(
                    !identity.contains(secret),
                    "credentials must not reach identity: {identity}"
                );
            }
        }
        assert_eq!(
            V8Connection::from_connection_string("unknown").snapshot_identity(dir.path()),
            None
        );
    }

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

    /// Серверы кластера читаются из `Srvr=` и из `/S`: список через запятую, без префикса
    /// протокола, в порядке записи; у файловой базы их нет.
    #[test]
    fn cluster_hosts_list_the_servers_of_srvr_and_s_in_order() {
        for (raw, expected) in [
            ("Srvr=srv:1541;Ref=demo", vec!["srv:1541"]),
            ("Srvr=\"[::1]\";Ref=\"demo\"", vec!["[::1]"]),
            (
                "Srvr='tcp://srv1:1541, srv2';Ref=demo",
                vec!["srv1:1541", "srv2"],
            ),
            ("Srvr=srv;Ref=demo;Locale=ru", vec!["srv"]),
            ("/S srv:1541\\demo", vec!["srv:1541"]),
        ] {
            assert_eq!(
                V8Connection::from_connection_string(raw).cluster_hosts(),
                expected,
                "{raw}"
            );
        }
        for raw in ["File=/tmp/ib", "/F /tmp/ib", "Srvr=host"] {
            assert!(
                V8Connection::from_connection_string(raw)
                    .cluster_hosts()
                    .is_empty(),
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
