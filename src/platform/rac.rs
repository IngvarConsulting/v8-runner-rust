//! `rac` — клиент администрирования кластера через `ras`: вопросы, которые нужны кластеру под
//! управлением раннера. Ответ `rac` — блоки строк `ключ : значение`, разделённые пустой строкой.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::platform::process::{
    ProcessError, ProcessExecutionPolicy, ProcessExecutor, ProcessInterruptionSafety,
    ProcessRequest, ProcessResult, ProcessRunner,
};
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
pub struct Credentials<'a> {
    pub user: Option<&'a str>,
    pub password: Option<&'a str>,
}

pub fn with_cluster_auth(
    mut request: ProcessRequest,
    credentials: Credentials<'_>,
) -> ProcessRequest {
    let address = request
        .args
        .pop()
        .expect("rac request ends with its address");
    append_auth(&mut request.args, "cluster", credentials);
    request.args.push(address);
    request
}

fn append_auth(args: &mut Vec<String>, level: &str, credentials: Credentials<'_>) {
    if let Some(user) = credentials.user {
        args.push(format!("--{level}-user={user}"));
        if let Some(password) = credentials.password {
            args.push(format!("--{level}-pwd={password}"));
        }
    }
}

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

pub fn probe_clusters(
    runner: &dyn ProcessRunner,
    rac: &Path,
    address: &str,
    policy: &ProcessExecutionPolicy,
) -> Result<Vec<Cluster>, ProcessError> {
    runner
        .run_with_policy(&cluster_list(rac, address), &policy.without_work())
        .map(|answer| {
            if answer.outcome().is_ok() {
                clusters(&answer.stdout)
            } else {
                Vec::new()
            }
        })
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

pub fn probe_infobases(
    runner: &dyn ProcessRunner,
    request: &ProcessRequest,
    policy: &ProcessExecutionPolicy,
) -> Result<crate::platform::process::ProcessResult, ProcessError> {
    runner.run_with_policy(request, &policy.without_work())
}

pub fn created_infobase(answer: &str) -> Option<Uuid> {
    (values(answer, "infobase").count() == 1)
        .then(|| uuid_field(answer, "infobase"))
        .flatten()
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

pub fn create_infobase(
    rac: &Path,
    ras: &str,
    base: &Registration<'_>,
    credentials: Credentials<'_>,
) -> ProcessRequest {
    let mut request = with_cluster_auth(register_infobase(rac, ras, base), credentials);
    request.args.insert(2, "--create-database".to_owned());
    request
        .args
        .insert(3, "--scheduled-jobs-deny=on".to_owned());
    request
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

pub fn session_terminate(
    rac: &Path,
    ras: &str,
    cluster: &str,
    session: &str,
    message: Option<&str>,
    credentials: Credentials<'_>,
) -> ProcessRequest {
    let mut args = vec![
        "session".to_owned(),
        "terminate".to_owned(),
        format!("--cluster={cluster}"),
        format!("--session={session}"),
    ];
    if let Some(message) = message {
        args.push(format!("--error-message={message}"));
    }
    append_auth(&mut args, "cluster", credentials);
    args.push(ras.to_owned());
    request(rac, args)
}

pub fn infobase_help(rac: &Path) -> ProcessRequest {
    request(rac, vec!["help".to_owned(), "infobase".to_owned()])
}

pub struct InfobaseTarget<'a> {
    pub cluster: &'a str,
    pub id: &'a str,
    pub cluster_auth: Credentials<'a>,
    pub infobase_auth: Credentials<'a>,
}

fn infobase_args(action: &str, target: &InfobaseTarget<'_>) -> Vec<String> {
    let mut args = vec![
        "infobase".to_owned(),
        action.to_owned(),
        format!("--cluster={}", target.cluster),
        format!("--infobase={}", target.id),
    ];
    append_auth(&mut args, "cluster", target.cluster_auth);
    append_auth(&mut args, "infobase", target.infobase_auth);
    args
}

pub fn infobase_info(rac: &Path, ras: &str, target: &InfobaseTarget<'_>) -> ProcessRequest {
    let mut args = infobase_args("info", target);
    args.push(ras.to_owned());
    request(rac, args)
}

pub struct LockUpdate<'a> {
    pub sessions_denied: bool,
    pub jobs_denied: Option<bool>,
    pub marker: &'a str,
    pub message: &'a str,
    pub permission_code: &'a str,
}

pub fn infobase_update(
    rac: &Path,
    ras: &str,
    target: &InfobaseTarget<'_>,
    lock: LockUpdate<'_>,
) -> ProcessRequest {
    let switch = |on| if on { "on" } else { "off" };
    let mut args = infobase_args("update", target);
    args.extend([
        format!("--sessions-deny={}", switch(lock.sessions_denied)),
        format!("--denied-parameter={}", lock.marker),
        format!("--denied-message={}", lock.message),
        format!("--permission-code={}", lock.permission_code),
    ]);
    if let Some(jobs) = lock.jobs_denied {
        args.push(format!("--scheduled-jobs-deny={}", switch(jobs)));
    }
    args.push(ras.to_owned());
    request(rac, args)
}

pub struct LockInfo {
    pub sessions_denied: bool,
    pub jobs_denied: bool,
    pub marker: String,
    pub message: String,
    pub code: String,
    pub scheduled: bool,
}

pub fn lock_info(answer: &str, id: &str) -> Option<LockInfo> {
    let block = blocks(answer).find(|block| field(block, "infobase") == Some(id))?;
    let switch = |key| match field(&block, key)? {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    Some(LockInfo {
        sessions_denied: switch("sessions-deny")?,
        jobs_denied: switch("scheduled-jobs-deny")?,
        marker: field(&block, "denied-parameter")?
            .trim_matches('"')
            .to_owned(),
        message: field(&block, "denied-message")?
            .trim_matches('"')
            .to_owned(),
        code: field(&block, "permission-code")?
            .trim_matches('"')
            .to_owned(),
        scheduled: ["denied-from", "denied-to"].into_iter().any(|key| {
            field(&block, key).is_some_and(|value| {
                !matches!(
                    value.trim_matches('"'),
                    "" | "0001-01-01T00:00:00" | "0001-01-01 00:00:00"
                )
            })
        }),
    })
}

pub struct Cluster {
    pub id: String,
    pub host: String,
    pub port: u16,
}

pub fn clusters(answer: &str) -> Vec<Cluster> {
    blocks(answer)
        .map(|block| {
            Some(Cluster {
                id: uuid_field(&block, "cluster")?.to_string(),
                host: field(&block, "host")?.to_owned(),
                port: field(&block, "port")?
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

fn uuid_field(block: &str, key: &str) -> Option<Uuid> {
    Uuid::parse_str(field(block, key)?)
        .ok()
        .filter(|id| !id.is_nil())
}

/// A successful query that can be used to decide whether it is safe to create a new base.
pub fn checked_infobase_id(answer: &str, name: &str) -> Option<Option<Uuid>> {
    let name = name.to_lowercase();
    let mut found = None;
    for block in blocks(answer) {
        let id = uuid_field(&block, "infobase")?;
        if infobase_name(&block)? == name && found.replace(id).is_some() {
            return None;
        }
    }
    Some(found)
}

/// Registration decisions need one well-formed cluster and an unambiguous base UUID.
pub(crate) fn confirmed_single_cluster(answer: &ProcessResult) -> Option<Cluster> {
    if !complete_response(answer) || !unique_keys(&answer.stdout, &["cluster", "host", "port"]) {
        return None;
    }
    let [cluster]: [Cluster; 1] = clusters(&answer.stdout).try_into().ok()?;
    Some(cluster)
}

pub(crate) fn confirmed_infobase_id(answer: &ProcessResult, name: &str) -> Option<Uuid> {
    if !complete_response(answer) || !unique_keys(&answer.stdout, &["infobase", "name"]) {
        return None;
    }
    checked_infobase_id(&answer.stdout, name)?
}

/// Read one cluster and its registered base using cancellable, read-only RAC requests.
pub(crate) fn probe_confirmed_registration(
    runner: &dyn ProcessRunner,
    rac: &Path,
    address: &str,
    reference: &str,
    policy: &ProcessExecutionPolicy,
) -> Result<Option<(Cluster, Uuid)>, ProcessError> {
    let policy = policy.without_work();
    let answer = runner.run_with_policy(&cluster_list(rac, address), &policy)?;
    let Some(cluster) = confirmed_single_cluster(&answer) else {
        return Ok(None);
    };
    let answer = runner.run_with_policy(&infobase_list(rac, address, &cluster.id), &policy)?;
    Ok(confirmed_infobase_id(&answer, reference).map(|infobase| (cluster, infobase)))
}

/// Идентификатор базы с этим именем из ответа `infobase summary list`; имена сравниваются, как
/// у [`lists_infobase`].
pub fn infobase_id(answer: &str, name: &str) -> Option<String> {
    let name = name.to_lowercase();
    blocks(answer).find_map(|block| {
        (infobase_name(&block)? == name)
            .then(|| field(&block, "infobase").map(str::to_owned))
            .flatten()
    })
}

fn infobase_name(block: &str) -> Option<String> {
    Some(field(block, "name")?.trim_matches('"').to_lowercase())
}

/// Адрес базы, выведенный из записи управляемого стенда для запуска агента.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedSessionTarget {
    pub local_path: PathBuf,
    pub project: PathBuf,
    pub name: String,
    pub reference: String,
    pub connection: String,
    pub cluster_user: Option<String>,
    pub cluster_password: Option<String>,
}

impl std::fmt::Debug for ManagedSessionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedSessionTarget")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl ManagedSessionTarget {
    /// Сессия агента исчезла, если в полном снимке этой базы нет сеансов Designer.
    pub(crate) fn designer_is_absent(&self, cancellation: &CancellationToken) -> bool {
        self.snapshot_excludes_designer(cancellation)
            .unwrap_or(false)
    }

    fn snapshot_excludes_designer(&self, cancellation: &CancellationToken) -> Option<bool> {
        use crate::platform::locator::UtilityType;
        use crate::platform::ragent::ras_address;
        use crate::platform::service_record::{running_cluster, running_ras};
        use crate::support::local_dir::LocalDir;

        let local = LocalDir::new(&self.local_path, &self.project);
        let pins = running_cluster(&local, &self.name)?;
        if !pins.registered || pins.ports.connection(&self.reference) != self.connection {
            return None;
        }
        let ras = running_ras(&local, &self.name)?;
        let address = ras_address(ras.port);
        let program = ras.bin.with_file_name(UtilityType::Rac.executable_name());
        let policy = ProcessExecutionPolicy::platform_step(
            Some(Duration::from_secs(3)),
            cancellation.clone(),
            ProcessInterruptionSafety::GracefulThenKill,
        );
        let run = |request| -> Option<ProcessResult> {
            if cancellation.is_cancelled() {
                return None;
            }
            let answer = ProcessExecutor.run_with_policy(&request, &policy).ok()?;
            complete_response(&answer).then_some(answer)
        };
        let answer = run(cluster_list(&program, &address))?;
        let cluster = confirmed_single_cluster(&answer)?;
        if cluster.port != pins.ports.regport {
            return None;
        }
        let auth = Credentials {
            user: self.cluster_user.as_deref(),
            password: self.cluster_password.as_deref(),
        };
        let answer = run(with_cluster_auth(
            infobase_list(&program, &address, &cluster.id),
            auth,
        ))?;
        let infobase = confirmed_infobase_id(&answer, &self.reference)?;
        let answer = run(with_cluster_auth(
            session_list(&program, &address, &cluster.id, &infobase.to_string()),
            auth,
        ))?;
        let still_running = running_ras(&local, &self.name)?;
        if still_running.pid != ras.pid
            || still_running.started != ras.started
            || still_running.port != ras.port
            || running_cluster(&local, &self.name)? != pins
        {
            return None;
        }
        Some(!cancellation.is_cancelled() && designer_absent(&answer, infobase))
    }
}

fn unique_keys(answer: &str, keys: &[&str]) -> bool {
    blocks(answer).all(|block| keys.iter().all(|key| values(&block, key).count() == 1))
}

fn complete_response(answer: &ProcessResult) -> bool {
    answer.outcome().is_ok()
        && answer.interruption.is_none()
        && answer.stderr.trim().is_empty()
        && answer.stdout.lines().all(|line| {
            line.trim().is_empty()
                || record_field(line).is_some_and(|(key, _)| {
                    let key = key.trim();
                    !key.is_empty()
                        && key
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
        })
}

fn designer_absent(answer: &ProcessResult, infobase: Uuid) -> bool {
    complete_response(answer)
        && checked_sessions_in(&answer.stdout, Some(infobase)).is_some_and(|sessions| {
            !sessions
                .iter()
                .any(|session| session.application.eq_ignore_ascii_case("Designer"))
        })
}

/// Один сеанс из ответа `session list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub number: Option<u32>,
    pub application: String,
    pub host: String,
    pub user: String,
    pub started: String,
    /// Сеанс спит (`hibernate : yes`): клиент пропал, например убит. Спящий сеанс обновлению
    /// конфигурации БД не мешает и платформа его в отказе не называет (замер #613).
    pub asleep: bool,
}

impl Session {
    fn parse(block: &str) -> Option<Self> {
        let value = |key| {
            field(block, key)
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned()
        };
        Some(Self {
            id: field(block, "session")?.to_owned(),
            number: field(block, "session-id").and_then(|value| value.parse().ok()),
            application: value("app-id"),
            host: value("host"),
            user: value("user-name"),
            started: value("started-at"),
            asleep: value("hibernate") == "yes",
        })
    }

    pub fn into_entry(self) -> crate::domain::sessions::SessionEntry {
        use crate::domain::sessions::{from_this_machine, protected_application, SessionEntry};
        SessionEntry {
            from_this_machine: from_this_machine(&self.host),
            computer: self.host,
            application: crate::platform::session_list::application_of_rac(&self.application),
            excluded: protected_application(&self.application),
            application_title: self.application,
            started: self.started,
            user: (!self.user.is_empty()).then_some(self.user),
            id: Some(self.id),
            number: self.number,
        }
    }
}

/// Сеансы из ответа `session list`: блок без `session` пропускается.
pub fn sessions(answer: &str) -> Vec<Session> {
    blocks(answer)
        .filter_map(|block| Session::parse(&block))
        .collect()
}

/// The entire session snapshot is needed before a selected UUID can be terminated.
pub fn checked_sessions(answer: &str) -> Option<Vec<Session>> {
    checked_sessions_in(answer, None)
}

fn checked_sessions_in(answer: &str, scope: Option<Uuid>) -> Option<Vec<Session>> {
    let mut ids = std::collections::HashSet::new();
    blocks(answer)
        .map(|block| {
            let id = uuid_field(&block, "session")?;
            let mut session = Session::parse(&block)?;
            if values(&block, "session").count() != 1
                || scope.is_some_and(|base| {
                    uuid_field(&block, "infobase") != Some(base)
                        || values(&block, "infobase").count() != 1
                        || values(&block, "app-id").count() != 1
                })
                || session.application.is_empty()
                || field(&block, "session-id").is_some() && session.number.is_none()
                || field(&block, "hibernate").is_some_and(|value| !matches!(value, "yes" | "no"))
                || !ids.insert(id)
            {
                return None;
            }
            session.id = id.to_string();
            Some(session)
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
    answer
        .lines()
        .filter_map(record_field)
        .filter_map(move |(name, value)| (name == key).then_some(value))
}

fn record_field(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once(" : ")?;
    Some((name.trim(), value.trim()))
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

    #[test]
    fn only_a_complete_snapshot_of_the_expected_base_can_exclude_designer() {
        use crate::platform::process::ProcessResult;

        let base = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let row = format!("session : 22222222-2222-2222-2222-222222222222\ninfobase : {base}\n");
        let scope = |exit_code, stdout: String, stderr: &str| ProcessResult {
            exit_code,
            stdout,
            stderr: stderr.to_owned(),
            interruption: None,
        };
        assert!(designer_absent(&scope(0, String::new(), ""), base));
        assert!(designer_absent(
            &scope(0, format!("{row}app-id : 1CV8C\n"), ""),
            base
        ));
        for stdout in [
            format!("{row}app-id : Designer\nhibernate : yes\n"),
            format!("{row}app-id : 1CV8C\napp-id : Designer\n"),
            format!("{row}app-id : 1CV8C\napp-id :Designer\n"),
            format!("{row}app-id : 1CV8C\n[v8-runner: 100 bytes of output omitted]\n"),
            row.clone(),
            format!("{row}app-id : 1CV8C\ninfobase : {base}\n"),
            format!(
                "{}app-id : 1CV8C\n",
                row.replace(&base.to_string(), "wrong")
            ),
        ] {
            assert!(!designer_absent(&scope(0, stdout, ""), base));
        }
        assert!(!designer_absent(&scope(1, String::new(), ""), base));
        assert!(!designer_absent(&scope(0, String::new(), "error"), base));
    }

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
        assert!(!found[0].asleep && !found[1].asleep);
        assert!(sessions("").is_empty());
        let asleep = sessions("session   : s3\napp-id    : 1CV8C\nhibernate : yes\n");
        assert!(asleep[0].asleep);
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
