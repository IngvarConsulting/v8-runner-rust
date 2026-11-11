//! `rac` — клиент администрирования кластера через `ras`: вопросы, которые нужны кластеру под
//! управлением раннера. Ответ `rac` — блоки строк `ключ : значение`, разделённые пустой строкой.

use std::path::Path;

use crate::platform::process::ProcessRequest;

/// База кластера для регистрации без создания БД.
pub struct Registration<'a> {
    pub cluster: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub server: &'a str,
    pub database: &'a str,
    pub user: Option<&'a str>,
    pub password: Option<&'a str>,
    pub locale: Option<&'a str>,
}

fn request(rac: &Path, args: Vec<String>) -> ProcessRequest {
    ProcessRequest {
        program: rac.to_path_buf(),
        args,
        workdir: None,
        stdout_log_path: None,
        stderr_log_path: None,
        startup_probe: None,
        windows_verbatim_args: Vec::new(),
    }
}

/// `rac cluster list`: готовность кластера и его идентификатор.
pub fn cluster_list(rac: &Path, ras: &str) -> ProcessRequest {
    request(
        rac,
        vec!["cluster".to_owned(), "list".to_owned(), ras.to_owned()],
    )
}

/// `rac infobase summary list`: какие базы зарегистрированы в кластере.
pub fn infobase_list(rac: &Path, ras: &str, cluster: &str) -> ProcessRequest {
    request(
        rac,
        vec![
            "infobase".to_owned(),
            "summary".to_owned(),
            "list".to_owned(),
            format!("--cluster={cluster}"),
            ras.to_owned(),
        ],
    )
}

/// `rac infobase create` без `--create-database`: регистрирует базу над уже существующей БД.
/// Пароль СУБД `rac` принимает только ключом; журнал его маскирует.
pub fn register_infobase(rac: &Path, ras: &str, base: &Registration<'_>) -> ProcessRequest {
    let mut args = vec![
        "infobase".to_owned(),
        "create".to_owned(),
        format!("--cluster={}", base.cluster),
        format!("--name={}", base.name),
        format!("--dbms={}", base.kind),
        format!("--db-server={}", base.server),
        format!("--db-name={}", base.database),
    ];
    if let Some(user) = base.user {
        args.push(format!("--db-user={user}"));
    }
    if let Some(password) = base.password {
        args.push(format!("--db-pwd={password}"));
    }
    if let Some(locale) = base.locale {
        args.push(format!("--locale={locale}"));
    }
    args.push(ras.to_owned());
    request(rac, args)
}

/// `rac infobase drop` без `--drop-database`: снимает регистрацию базы, БД остаётся. У базы с
/// пользователями `rac` требует пользователя базы; пароль он принимает только ключом.
pub fn unregister_infobase(
    rac: &Path,
    ras: &str,
    cluster: &str,
    infobase: &str,
    user: Option<&str>,
    password: Option<&str>,
) -> ProcessRequest {
    let mut args = vec![
        "infobase".to_owned(),
        "drop".to_owned(),
        format!("--cluster={cluster}"),
        format!("--infobase={infobase}"),
    ];
    if let Some(user) = user {
        args.push(format!("--infobase-user={user}"));
    }
    if let Some(password) = password {
        args.push(format!("--infobase-pwd={password}"));
    }
    args.push(ras.to_owned());
    request(rac, args)
}

/// `rac session list --infobase`: сеансы одной базы кластера.
pub fn session_list(rac: &Path, ras: &str, cluster: &str, infobase: &str) -> ProcessRequest {
    request(
        rac,
        vec![
            "session".to_owned(),
            "list".to_owned(),
            format!("--cluster={cluster}"),
            format!("--infobase={infobase}"),
            ras.to_owned(),
        ],
    )
}

/// Идентификатор базы с этим именем из ответа `infobase summary list`; имена сравниваются, как
/// у [`lists_infobase`].
pub fn infobase_id(answer: &str, name: &str) -> Option<String> {
    let name = name.to_lowercase();
    blocks(answer).find_map(|block| {
        let listed = field(&block, "name")?.trim_matches('"').to_lowercase();
        (listed == name)
            .then(|| field(&block, "infobase").map(str::to_owned))
            .flatten()
    })
}

/// Один сеанс из ответа `session list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub application: String,
    pub host: String,
    pub user: String,
    pub started: String,
}

/// Сеансы из ответа `session list`: блок без `session` пропускается.
pub fn sessions(answer: &str) -> Vec<Session> {
    blocks(answer)
        .filter_map(|block| {
            let value = |key| {
                field(&block, key)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned()
            };
            Some(Session {
                id: field(&block, "session")?.to_owned(),
                application: value("app-id"),
                host: value("host"),
                user: value("user-name"),
                started: value("started-at"),
            })
        })
        .collect()
}

/// Блоки ответа: строки, разделённые пустой строкой.
fn blocks(answer: &str) -> impl Iterator<Item = String> + '_ {
    let mut lines = answer.lines().peekable();
    std::iter::from_fn(move || {
        while lines.peek().is_some_and(|line| line.trim().is_empty()) {
            lines.next();
        }
        lines.peek()?;
        let mut block = String::new();
        while let Some(line) = lines.next_if(|line| !line.trim().is_empty()) {
            block.push_str(line);
            block.push('\n');
        }
        Some(block)
    })
}

/// Значение ключа в одном блоке.
fn field<'a>(block: &'a str, key: &'a str) -> Option<&'a str> {
    values(block, key).next()
}

/// Значения ключа во всех блоках ответа. `rac` пишет строку `ключ<пробелы> : значение`, и делит её
/// именно ` : `: двоеточие внутри значения — например, `host:port` — строку не режет.
pub fn values<'a>(answer: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    answer.lines().filter_map(move |line| {
        let (name, value) = line.split_once(" : ")?;
        (name.trim() == key).then(|| value.trim())
    })
}

/// Идентификатор первого кластера из ответа `cluster list`.
pub fn first_cluster(answer: &str) -> Option<String> {
    values(answer, "cluster")
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Есть ли база с этим именем в ответе `infobase summary list`. Платформа сравнивает имена баз без
/// учёта регистра, и кириллицы тоже.
pub fn lists_infobase(answer: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    // Имя с пробелом `rac` пишет в кавычках.
    values(answer, "name").any(|value| value.trim_matches('"').to_lowercase() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_infobase_id_and_its_sessions_are_read_from_the_answers() {
        let summary = "infobase : 11\nname     : other\n\ninfobase : 22\nname     : \"Демо\"\n";
        assert_eq!(infobase_id(summary, "демо").as_deref(), Some("22"));
        assert_eq!(infobase_id(summary, "missing"), None);
        let list = "session      : s1\r\nuser-name    : Иванов\r\nhost         : PC1\r\napp-id       : 1CV8C\r\nstarted-at   : 2026-11-11T02:00:00\r\n\r\nsession      : s2\r\napp-id       : Designer\r\nhost         : PC2\r\n";
        let found = sessions(list);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].application, "1CV8C");
        assert_eq!(found[0].user, "Иванов");
        assert_eq!(found[1].user, "");
        assert!(sessions("").is_empty());
    }

    #[test]
    fn an_unregistration_keeps_the_database() {
        let request =
            unregister_infobase(Path::new("rac.exe"), "127.0.0.1:1", "c", "i", None, None);
        assert_eq!(
            request.args,
            [
                "infobase",
                "drop",
                "--cluster=c",
                "--infobase=i",
                "127.0.0.1:1"
            ]
        );
        let request = unregister_infobase(
            Path::new("rac.exe"),
            "127.0.0.1:1",
            "c",
            "i",
            Some("admin"),
            Some("secret"),
        );
        assert!(request.args.contains(&"--infobase-user=admin".to_owned()));
        assert!(request.args.contains(&"--infobase-pwd=secret".to_owned()));
        assert!(!request.args.iter().any(|arg| arg.contains("drop-database")));
    }

    #[test]
    fn a_listed_name_matches_without_case_and_quotes() {
        let answer = "infobase : x\nname     : \"Учёт Склада\"\n\ninfobase : y\nname     : Демо\n";
        assert!(lists_infobase(answer, "учёт склада"));
        assert!(lists_infobase(answer, "ДЕМО"));
        assert!(!lists_infobase(answer, "другая"));
    }

    const CLUSTERS: &str = "cluster                       : 6c5a8a62-3a8c-4e4b-b0a6-2f4c4b1f1d10\nhost                          : 127.0.0.1\nport                          : 24001\nname                          : \"Local cluster\"\n\n";

    #[test]
    fn the_first_cluster_is_read_from_the_answer() {
        assert_eq!(
            first_cluster(CLUSTERS).as_deref(),
            Some("6c5a8a62-3a8c-4e4b-b0a6-2f4c4b1f1d10")
        );
        assert_eq!(first_cluster(""), None);
    }

    #[test]
    fn a_registered_infobase_is_found_by_its_name() {
        let answer =
            "infobase : 1\nname     : Demo\ndescr    :\n\ninfobase : 2\nname     : other\n";
        assert!(lists_infobase(answer, "demo"));
        assert!(!lists_infobase(answer, "dem"));
    }

    #[test]
    fn a_registration_creates_no_database_and_ends_with_the_ras_address() {
        let request = register_infobase(
            Path::new("rac.exe"),
            "127.0.0.1:24002",
            &Registration {
                cluster: "c",
                name: "demo",
                kind: "PostgreSQL",
                server: "127.0.0.1 port=24432",
                database: "demo_data",
                user: Some("postgres"),
                password: Some("pw"),
                locale: Some("ru"),
            },
        );
        assert!(!request.args.iter().any(|arg| arg == "--create-database"));
        assert!(request
            .args
            .contains(&"--db-server=127.0.0.1 port=24432".to_owned()));
        assert_eq!(
            request.args.last().map(String::as_str),
            Some("127.0.0.1:24002")
        );
    }
}
