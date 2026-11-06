//! Сборка PostgreSQL для 1С: аргументы утилит, проба и файлы каталога данных. Что запускать и в
//! каком порядке, решает `use_cases/managed_stand`.

use std::path::{Path, PathBuf};

use crate::platform::process::ProcessRequest;
use crate::platform::service::ServiceRequest;

/// Пользователь СУБД, которого создаёт `initdb`.
pub const SUPERUSER: &str = "postgres";

/// Локаль `initdb` на Windows: с ней `datcollate` — `Russian_Russia.1251`, и платформа принимает БД.
/// Форму `ru-RU` платформа отвергает (замер #534).
const WINDOWS_LOCALE: &str = "Russian_Russia";

/// Каталог `bin` сборки, найденный по `tools.postgres.path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresBuild {
    bin: PathBuf,
}

impl PostgresBuild {
    /// Сборка из каталога `path` или его `bin`; `None`, если `pg_ctl` там нет.
    pub fn locate(path: &Path) -> Option<Self> {
        [path.join("bin"), path.to_path_buf()]
            .into_iter()
            .find(|bin| bin.join(executable("pg_ctl")).is_file())
            .map(|bin| Self { bin })
    }

    /// Сборка по записанному каталогу `bin`, без поиска.
    pub fn at(bin: PathBuf) -> Self {
        Self { bin }
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    fn request(&self, tool: &str, args: Vec<String>) -> ProcessRequest {
        ProcessRequest {
            program: self.bin.join(executable(tool)),
            args,
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
            windows_verbatim_args: Vec::new(),
        }
    }

    pub fn version(&self) -> ProcessRequest {
        self.request("pg_ctl", vec!["--version".to_owned()])
    }

    pub fn initdb(&self, data: &Path, password_file: &Path) -> ProcessRequest {
        self.request(
            "initdb",
            vec![
                "-D".to_owned(),
                data.display().to_string(),
                "-U".to_owned(),
                SUPERUSER.to_owned(),
                "-E".to_owned(),
                "UTF8".to_owned(),
                format!("--locale={WINDOWS_LOCALE}"),
                "-A".to_owned(),
                "scram-sha-256".to_owned(),
                format!("--pwfile={}", password_file.display()),
            ],
        )
    }

    /// `pg_ctl start` без ожидания: запускает `postgres` и выходит. Именно `pg_ctl`, потому что
    /// под администратором сам `postgres` не стартует, а `pg_ctl` снимает права. Готовность
    /// проверяет `is_ready`.
    pub fn start(&self, data: &Path, log: &Path, port: u16) -> ServiceRequest {
        ServiceRequest {
            program: self.bin.join(executable("pg_ctl")),
            args: vec![
                "start".to_owned(),
                "-W".to_owned(),
                "-D".to_owned(),
                data.display().to_string(),
                "-l".to_owned(),
                log.display().to_string(),
                "-o".to_owned(),
                format!("-p {port} -c listen_addresses=127.0.0.1"),
            ],
            // Иначе служба держала бы каталог, из которого её запустили.
            workdir: data.parent().map(Path::to_path_buf),
            log_path: log.with_extension("start.log"),
        }
    }

    pub fn stop(&self, data: &Path) -> ProcessRequest {
        self.request(
            "pg_ctl",
            vec![
                "stop".to_owned(),
                "-D".to_owned(),
                data.display().to_string(),
                "-m".to_owned(),
                "fast".to_owned(),
                "-w".to_owned(),
                "-t".to_owned(),
                "60".to_owned(),
            ],
        )
    }

    pub fn is_ready(&self, port: u16) -> ProcessRequest {
        self.request(
            "pg_isready",
            vec![
                "-h".to_owned(),
                "127.0.0.1".to_owned(),
                "-p".to_owned(),
                port.to_string(),
                "-q".to_owned(),
            ],
        )
    }
}

fn executable(tool: &str) -> String {
    format!("{tool}{}", std::env::consts::EXE_SUFFIX)
}

/// Значение `--db-server` для этого порта.
pub fn server_address(port: u16) -> String {
    format!("127.0.0.1 port={port}")
}

/// Создан ли каталог данных.
pub fn data_dir_is_initialized(data: &Path) -> bool {
    data.join("PG_VERSION").is_file()
}

/// Pid главного процесса: первая строка `postmaster.pid`. Сам `pg_ctl` к этому времени завершён.
pub fn postmaster_pid(data: &Path) -> Option<u32> {
    std::fs::read_to_string(data.join("postmaster.pid"))
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Версия из ответа `pg_ctl --version`: `pg_ctl (PostgreSQL) 17.11 ...` → `17.11`.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_uses_the_platform_form_and_the_settings_stay_on_the_loopback() {
        assert_eq!(server_address(24432), "127.0.0.1 port=24432");
        let start = PostgresBuild::at(PathBuf::from("bin")).start(
            Path::new("root/data"),
            Path::new("log"),
            24432,
        );
        assert!(start
            .args
            .contains(&"-p 24432 -c listen_addresses=127.0.0.1".to_owned()));
        assert_eq!(start.workdir.as_deref(), Some(Path::new("root")));
    }

    #[test]
    fn the_version_is_the_first_number_of_the_answer() {
        assert_eq!(
            parse_version("pg_ctl (PostgreSQL) 17.11 (Postgres Pro 1C)\n").as_deref(),
            Some("17.11")
        );
        assert_eq!(parse_version("nothing"), None);
    }

    #[test]
    fn the_pid_is_the_first_line_of_the_postmaster_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(postmaster_pid(dir.path()), None);
        std::fs::write(dir.path().join("postmaster.pid"), "40112\r\nE:/data\r\n").unwrap();
        assert_eq!(postmaster_pid(dir.path()), Some(40112));
    }

    #[test]
    fn initdb_takes_the_password_from_a_file_and_never_as_an_argument_value() {
        let build = PostgresBuild::at(PathBuf::from("bin"));
        let request = build.initdb(Path::new("data"), Path::new("password"));
        assert!(request.args.contains(&"--pwfile=password".to_owned()));
        assert!(request.args.contains(&"scram-sha-256".to_owned()));
        assert!(request.args.contains(&"--locale=Russian_Russia".to_owned()));
    }
}
