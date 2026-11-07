//! Автономный сервер `ibsrv`, которого поднимает раннер: ключи запуска. Что запускать и в каком
//! порядке, решает `use_cases/managed_stand`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::platform::service::ServiceRequest;

/// Сколько портов в диапазоне прямого шлюза.
pub const DIRECT_RANGE_WIDTH: u16 = 16;

/// Порты сервера. Закрепляются при первом запуске: адрес `Srvr=` и память о базе не меняются от
/// запуска к запуску.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerPorts {
    pub direct: u16,
    pub ssh: u16,
    pub http: u16,
    pub range_start: u16,
}

impl ServerPorts {
    /// Порты блоком от `first`: блоки разных копий выводятся из пути и не пересекаются.
    pub fn from_first(first: u16) -> Self {
        Self {
            direct: first,
            ssh: first + 1,
            http: first + 2,
            range_start: first + 3,
        }
    }

    pub fn all(self) -> impl Iterator<Item = u16> {
        [self.direct, self.ssh, self.http]
            .into_iter()
            .chain(self.range_start..self.range_start + DIRECT_RANGE_WIDTH)
    }

    /// Строка соединения прямого шлюза.
    pub fn connection(self, name: &str) -> String {
        format!("Srvr=127.0.0.1:{};Ref={name}", self.direct)
    }

    /// Адрес SSH-шлюза для `standalone.gate`.
    pub fn gate(self) -> String {
        format!("127.0.0.1:{}", self.ssh)
    }

    pub fn http_url(self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.http)
    }
}

/// Где база сервера.
pub enum ServerStorage<'a> {
    File(&'a Path),
    Dbms(DbmsTarget<'a>),
}

/// База в СУБД: ключи `--dbms`, `--db-server`, `--db-name`, `--db-user`.
pub struct DbmsTarget<'a> {
    pub kind: &'a str,
    pub server: &'a str,
    pub name: &'a str,
    pub user: Option<&'a str>,
    pub password: Option<&'a str>,
}

/// Запуск сервера. Без `--enable-extended-designer-features`: с ним сервер не сообщает о готовности,
/// и `ibcmd --pid` не подключается (замер #82). Пароль СУБД уходит через stdin ключом `-W`: в командной
/// строке долгоживущего процесса его видели бы все пользователи машины.
pub fn start_server(
    ibsrv: &Path,
    data: &Path,
    storage: &ServerStorage<'_>,
    name: &str,
    ports: ServerPorts,
    host_key: &Path,
    log: &Path,
) -> ServiceRequest {
    let mut args = vec![format!("--data={}", data.display())];
    let mut stdin = None;
    match storage {
        ServerStorage::File(db_path) => args.push(format!("--db-path={}", db_path.display())),
        ServerStorage::Dbms(target) => {
            args.push(format!("--dbms={}", target.kind));
            args.push(format!("--db-server={}", target.server));
            args.push(format!("--db-name={}", target.name));
            if let Some(user) = target.user {
                args.push(format!("--db-user={user}"));
            }
            if let Some(password) = target.password {
                args.push("-W".to_owned());
                stdin = Some(password.to_owned());
            }
        }
    }
    args.extend([
        format!("--name={name}"),
        format!("--http-port={}", ports.http),
        format!("--direct-regport={}", ports.direct),
        format!(
            "--direct-range={}:{}",
            ports.range_start,
            ports.range_start + DIRECT_RANGE_WIDTH - 1
        ),
        "--enable-ssh-gate".to_owned(),
        format!("--ssh-port={}", ports.ssh),
        format!("--ssh-host-key={}", host_key.display()),
    ]);
    ServiceRequest {
        program: ibsrv.to_path_buf(),
        args,
        workdir: data.parent().map(Path::to_path_buf),
        log_path: log.to_path_buf(),
        stdin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ports_of_a_block_do_not_overlap_and_name_the_addresses() {
        let ports = ServerPorts::from_first(24000);
        let all: Vec<u16> = ports.all().collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
        assert_eq!(ports.connection("demo"), "Srvr=127.0.0.1:24000;Ref=demo");
        assert_eq!(ports.gate(), "127.0.0.1:24001");
    }

    #[test]
    fn the_server_starts_with_the_ssh_gate_and_without_the_extended_designer_features() {
        let request = start_server(
            Path::new("ibsrv"),
            Path::new("root/data"),
            &ServerStorage::File(Path::new("ib")),
            "demo",
            ServerPorts::from_first(24000),
            Path::new("key"),
            Path::new("log"),
        );
        assert!(request.args.contains(&"--enable-ssh-gate".to_owned()));
        assert!(request.args.contains(&"--direct-regport=24000".to_owned()));
        assert!(request
            .args
            .contains(&"--direct-range=24003:24018".to_owned()));
        assert!(!request
            .args
            .iter()
            .any(|arg| arg.contains("extended-designer")));
        assert_eq!(request.workdir.as_deref(), Some(Path::new("root")));
    }

    #[test]
    fn the_dbms_password_goes_through_stdin_and_never_through_an_argument() {
        let request = start_server(
            Path::new("ibsrv"),
            Path::new("root/data"),
            &ServerStorage::Dbms(DbmsTarget {
                kind: "PostgreSQL",
                server: "127.0.0.1 port=24432",
                name: "demo_data",
                user: Some("postgres"),
                password: Some("secret-value"),
            }),
            "demo",
            ServerPorts::from_first(24000),
            Path::new("key"),
            Path::new("log"),
        );
        assert!(request.args.contains(&"-W".to_owned()));
        assert!(request
            .args
            .contains(&"--db-server=127.0.0.1 port=24432".to_owned()));
        assert!(!request.args.iter().any(|arg| arg.contains("secret-value")));
        assert_eq!(request.stdin.as_deref(), Some("secret-value"));
    }
}
