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

    /// Return the file-based infobase path when connection string contains `File=...`.
    pub fn file_path(&self) -> Option<&str> {
        if self.raw.starts_with('/') || self.raw.starts_with('-') {
            return file_path_from_args(&self.connection_args);
        }

        self.raw.split(';').find_map(|part| {
            let part = part.trim();
            let lower = part.to_lowercase();
            if lower.starts_with("file=") {
                Some(&part[5..])
            } else {
                None
            }
        })
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

        let mut server = None;
        let mut reference = None;
        for part in self.raw.split(';') {
            let Some((key, value)) = part.split_once('=') else {
                return false;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "srvr" => server = Some(value.trim()),
                "ref" => reference = Some(value.trim()),
                _ => {}
            }
        }
        server.is_some_and(|value| !value.is_empty())
            && reference.is_some_and(|value| !value.is_empty())
    }

    /// Returns a stable file-based infobase connection string when available.
    pub fn create_infobase_arg(&self) -> Option<String> {
        self.file_path()
            .map(|path| format!("File='{}'", path.replace('\'', "''")))
    }
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
