//! Сервер отладки 1С `dbgs`, которого поднимает раннер: где он лежит, с какими ключами
//! запускается, как назвал свой адрес и отвечает ли его обработчик протокола `rdbg`. Когда
//! поднимать и гасить, решает `use_cases/managed_stand`.
//!
//! Поведение платформы — замер «Сервер отладки `dbgs`» в
//! `references/1c/confirmed-runtime-measurements.md`.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::platform::service::ServiceRequest;

/// Имя исполняемого файла сервера отладки на этой системе.
pub const fn executable_name() -> &'static str {
    if cfg!(windows) {
        "dbgs.exe"
    } else {
        "dbgs"
    }
}

/// Сервер отладки той установки, из которой запускается клиент: он лежит рядом с ним.
pub fn beside(client: &Path) -> PathBuf {
    client.parent().map_or_else(
        || PathBuf::from(executable_name()),
        |bin| bin.join(executable_name()),
    )
}

/// Какой порт слушать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPort {
    /// Этот порт, и никакой другой: занят — сервер выходит с кодом 1.
    Exact(u16),
    /// Первый свободный порт окна: сервер выбирает его сам и называет в файле `--notify`.
    Window { first: u16, last: u16 },
}

/// Ключи запуска: только петля, адрес — в файл `notify`. `owner` — процесс, с концом которого
/// сервер выходит сам (`--ownerPID`).
pub fn start_request(
    dbgs: &Path,
    port: DebugPort,
    notify: &Path,
    owner: Option<u32>,
    log: &Path,
) -> ServiceRequest {
    let mut args = vec![format!("--addr={}", Ipv4Addr::LOCALHOST)];
    args.push(match port {
        DebugPort::Exact(port) => format!("--port={port}"),
        DebugPort::Window { first, last } => format!("--portRange={first}:{last}"),
    });
    args.push(format!("--notify={}", notify.display()));
    if let Some(owner) = owner {
        args.push(format!("--ownerPID={owner}"));
    }
    ServiceRequest {
        program: dbgs.to_path_buf(),
        args,
        workdir: notify.parent().map(Path::to_path_buf),
        log_path: log.to_path_buf(),
        stdin: None,
    }
}

/// Порт из файла `--notify`: `127.0.0.1:1561` в UTF-16 LE с BOM. Читается и UTF-8. `None` —
/// файла ещё нет или он дописан не до конца.
pub fn notified_port(notify: &Path) -> Option<u16> {
    parse_notify(&std::fs::read(notify).ok()?)
}

fn parse_notify(bytes: &[u8]) -> Option<u16> {
    let text = match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16(&units).ok()?
        }
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).ok()?,
        other => String::from_utf8(other.to_vec()).ok()?,
    };
    let (_, port) = crate::support::authority::host_and_port_of_authority(text.trim())?;
    port.filter(|port| *port != 0)
}

/// Отвечает ли на порту обработчик протокола `rdbg`. Запрос `getDbgTargets` без
/// зарегистрированного отладчика ничего не меняет у сервера, а ответ 400 с исключением
/// `virtual-resource-system` даёт только этот обработчик: просто открытый порт, чужой путь
/// (404) или запрос без `Content-Type: application/xml` (406) готовности не подтверждают.
/// Прокси окружения не участвует: адрес — петля.
pub fn rdbg_answers(port: u16) -> bool {
    probe_rdbg(&url(port))
}

/// Probe the direct HTTP address using the same read-only request as local dbgs readiness.
pub(crate) fn probe_rdbg(address: &str) -> bool {
    // Один клиент на процесс: ожидание опрашивает сервер каждые 200 мс, а клиент `blocking`
    // держит свой поток с рантаймом.
    static CLIENT: std::sync::OnceLock<Option<reqwest::blocking::Client>> =
        std::sync::OnceLock::new();
    let Some(client) = CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .ok()
        })
        .as_ref()
    else {
        return false;
    };
    let url = format!("{address}/e1crdbg/rdbg?cmd=getDbgTargets");
    let Ok(response) = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(Vec::new())
        .send()
    else {
        return false;
    };
    if response.status() != reqwest::StatusCode::BAD_REQUEST {
        return false;
    }
    response.text().is_ok_and(|body| is_rdbg_refusal(&body))
}

/// Отказ обработчика `rdbg`: исключение `virtual-resource-system`. С `Accept: application/json`
/// оно приходит в JSON, без `Accept` — в XML; признак один и тот же.
fn is_rdbg_refusal(body: &str) -> bool {
    body.contains("virtual-resource-system") && body.contains("Exception")
}

/// Адрес сервера отладки для клиента (`/DEBUGGERURL`) и отладчика.
pub fn url(port: u16) -> String {
    format!("http://{}:{port}", Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn the_notify_file_names_the_port_in_utf16_with_a_bom() {
        assert_eq!(parse_notify(&utf16("127.0.0.1:1561")), Some(1561));
        assert_eq!(parse_notify(b"127.0.0.1:27650\n"), Some(27650));
        assert_eq!(parse_notify(&utf16("127.0.0.1:")), None);
        assert_eq!(parse_notify(&[0xFF, 0xFE, 0x31]), None);
        assert_eq!(parse_notify(b""), None);
    }

    #[test]
    fn the_request_listens_on_the_loopback_and_names_its_owner() {
        let request = start_request(
            Path::new("/opt/1cv8/bin/dbgs"),
            DebugPort::Window {
                first: 10_000,
                last: 10_031,
            },
            Path::new("/w/debug-server/s/notify"),
            Some(42),
            Path::new("/w/debug-server/s/dbgs.log"),
        );
        assert_eq!(
            request.args,
            [
                "--addr=127.0.0.1",
                "--portRange=10000:10031",
                "--notify=/w/debug-server/s/notify",
                "--ownerPID=42",
            ]
        );
        let exact = start_request(
            Path::new("dbgs"),
            DebugPort::Exact(1560),
            Path::new("n"),
            None,
            Path::new("l"),
        );
        assert_eq!(exact.args[1], "--port=1560");
        assert!(!exact.args.iter().any(|arg| arg.starts_with("--ownerPID")));
    }

    #[test]
    fn only_the_rdbg_refusal_counts_as_an_answer() {
        let refusal = "\u{feff}{\"#exception\":\"{http://v8.1c.ru/8.2/virtual-resource-system}Exception\",\"exception\":{\"reason\":400}}";
        assert!(is_rdbg_refusal(refusal));
        let xml = "\u{feff}<?xml version=\"1.0\"?><exception xmlns=\"http://v8.1c.ru/8.2/virtual-resource-system\" xsi:type=\"Exception\" reason=\"400\"/>";
        assert!(is_rdbg_refusal(xml));
        assert!(!is_rdbg_refusal("<html>bad request</html>"));
    }

    #[test]
    fn the_server_lies_beside_the_client() {
        let client = Path::new("C:/1cv8/8.3.27.2342/bin").join("1cv8c.exe");
        assert_eq!(
            beside(&client),
            Path::new("C:/1cv8/8.3.27.2342/bin").join(executable_name())
        );
    }
}
