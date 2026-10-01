use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode, Url};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::support::authority::host_of_url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Сколько молчания считать застреванием.
///
/// Это не бюджет на загрузку: большой архив качается долго и имеет право. Мерится
/// тишина — промежуток, в который не пришло ни байта. Зеркало, принявшее соединение
/// и замолчавшее, иначе держит команду столько, сколько она согласна ждать, а без
/// срока на команду — навсегда.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(test)]
thread_local! {
    /// Шов для тестов: ждать минуту тишины в прогоне нельзя, а проверять надо именно её.
    static READ_IDLE_OVERRIDE: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

fn read_idle_timeout() -> Duration {
    #[cfg(test)]
    {
        if let Some(value) = READ_IDLE_OVERRIDE.with(std::cell::Cell::get) {
            return value;
        }
    }
    READ_IDLE_TIMEOUT
}
const RETRY_ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("HTTP client setup failed: {0}")]
    Client(reqwest::Error),

    #[error("HTTP GET {url} failed: {source}")]
    Request { url: String, source: reqwest::Error },

    #[error("HTTP GET {url} returned status {status}")]
    Status { url: String, status: StatusCode },

    #[error("HTTP response read failed for {url}: {source}")]
    Read { url: String, source: reqwest::Error },

    #[error(
        "HTTP response for {url} exceeds maximum download size {max_bytes} bytes: {size_bytes} bytes"
    )]
    ResponseTooLarge {
        url: String,
        size_bytes: u64,
        max_bytes: u64,
    },

    #[error("HTTP download timed out after {timeout_ms}ms")]
    TimedOut { timeout_ms: u64 },

    #[error("HTTP download was cancelled")]
    Cancelled,

    #[error("response is not UTF-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("refusing to download over a plain-text connection: {url}")]
    InsecureScheme { url: String },

    #[error("not a usable download address: {url}")]
    UnusableUrl { url: String },

    #[error("failed to build the runtime for the download: {0}")]
    Runtime(std::io::Error),
}

pub fn get_text(
    url: &str,
    timeout: Option<Duration>,
    cancellation: &CancellationToken,
) -> Result<String, DownloadError> {
    let bytes = get_bytes(url, timeout, cancellation)?;
    String::from_utf8(bytes).map_err(DownloadError::InvalidUtf8)
}

/// Адрес, по которому допустимо качать.
///
/// Всё, что приезжает извне и потом исполняется — расширения `.cfe`, архивы исходников, —
/// должно ехать по TLS: без него содержимое выбирает любой посредник. Исключение одно и
/// узкое: петлевой адрес, потому что фикстуры тестов поднимают обычный HTTP на
/// `127.0.0.1`, и там посредника нет по построению.
///
/// Петля опознаётся разбором адреса, а не сравнением подстрок: `127.evil.com` и
/// `127.0.0.1@evil.com` начинаются как петлевой адрес, но ведут наружу.
fn ensure_transport_is_protected(url: &Url) -> Result<(), DownloadError> {
    let scheme = url.scheme();
    if scheme.eq_ignore_ascii_case("https") {
        return Ok(());
    }
    let refuse = || DownloadError::InsecureScheme {
        url: url.as_str().to_owned(),
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(refuse());
    }
    match host_of_url(url) {
        Some(host) if host.is_loopback() => Ok(()),
        _ => Err(refuse()),
    }
}

pub fn get_bytes(
    url: &str,
    timeout: Option<Duration>,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, DownloadError> {
    let url = Url::parse(url).map_err(|_| DownloadError::UnusableUrl {
        url: url.to_owned(),
    })?;
    ensure_transport_is_protected(&url)?;
    if timeout.is_some_and(|value| value.is_zero()) {
        return Err(DownloadError::TimedOut { timeout_ms: 0 });
    }

    // Загрузка идёт на асинхронном клиенте ради `read_timeout`: у блокирующего есть
    // только общий срок, то есть снова часы, а мерить надо тишину. Вызывающий об этом
    // не знает — снаружи функция осталась обычной.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DownloadError::Runtime)?;

    runtime.block_on(async move {
        let started = Instant::now();
        // Каждая попытка кончается либо байтами, либо своей ошибкой — после последней
        // подставлять нечего, и отмену из ниоткуда ответ не назовёт.
        let mut attempt = 1;
        loop {
            ensure_not_cancelled(cancellation)?;
            let request_timeout = remaining_budget(timeout, started)?;
            let client = build_client(request_timeout)?;

            match download_once(&client, &url, timeout, started, cancellation).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) if attempt < RETRY_ATTEMPTS && error.is_retryable() => {
                    sleep_before_retry(cancellation, timeout, started).await?;
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    })
}

fn build_client(timeout: Option<Duration>) -> Result<Client, DownloadError> {
    let connect_timeout = timeout
        .map(|value| value.min(CONNECT_TIMEOUT))
        .unwrap_or(CONNECT_TIMEOUT);
    let mut builder = Client::builder()
        .connect_timeout(connect_timeout)
        // Тишина в теле ответа ограничена всегда, даже когда бюджета у команды нет.
        .read_timeout(read_idle_timeout())
        .user_agent("v8-runner")
        // Правило схемы проверяется и на каждом переходе: иначе `https`, отвечающий
        // редиректом на `http`, тихо уводил бы загрузку с TLS.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            match ensure_transport_is_protected(attempt.url()) {
                Ok(()) => attempt.follow(),
                // Не `stop()`: остановка выдала бы отказ за обычный ответ `302`, и
                // тот, кто разбирается, почему загрузка не идёт, не узнал бы, что
                // раннер отказался уходить с TLS.
                Err(_) => attempt.error("refusing to follow a redirect off TLS"),
            }
        }));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(DownloadError::Client)
}

async fn download_once(
    client: &Client,
    url: &Url,
    timeout: Option<Duration>,
    started: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, DownloadError> {
    ensure_not_cancelled(cancellation)?;
    let url_text = url.as_str().to_owned();
    let response = client
        .get(url.clone())
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|source| DownloadError::Request {
            url: url_text.clone(),
            source,
        })?;

    if !response.status().is_success() {
        return Err(DownloadError::Status {
            url: url_text,
            status: response.status(),
        });
    }

    let content_length = response.content_length();
    if let Some(size_bytes) = content_length {
        ensure_allowed_download_size(&url_text, size_bytes)?;
    }
    let capacity = content_length
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    let mut response = response;

    loop {
        ensure_not_cancelled(cancellation)?;
        let _ = remaining_budget(timeout, started)?;
        // Кусок за куском: `read_timeout` клиента отмеряет тишину между ними, а не
        // общую длительность, поэтому большой архив качается столько, сколько качается.
        let chunk = response
            .chunk()
            .await
            .map_err(|source| DownloadError::Read {
                url: url_text.clone(),
                source,
            })?;
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX);
        ensure_allowed_download_size(&url_text, next_len)?;
        bytes.extend_from_slice(&chunk);
    }
}

fn ensure_allowed_download_size(url: &str, size_bytes: u64) -> Result<(), DownloadError> {
    if size_bytes > MAX_DOWNLOAD_BYTES {
        Err(DownloadError::ResponseTooLarge {
            url: url.to_owned(),
            size_bytes,
            max_bytes: MAX_DOWNLOAD_BYTES,
        })
    } else {
        Ok(())
    }
}

fn remaining_budget(
    timeout: Option<Duration>,
    started: Instant,
) -> Result<Option<Duration>, DownloadError> {
    let Some(limit) = timeout else {
        return Ok(None);
    };
    limit
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .map(Some)
        .ok_or_else(|| DownloadError::TimedOut {
            timeout_ms: limit.as_millis() as u64,
        })
}

async fn sleep_before_retry(
    cancellation: &CancellationToken,
    timeout: Option<Duration>,
    started: Instant,
) -> Result<(), DownloadError> {
    let delay = remaining_budget(timeout, started)?
        .map(|remaining| remaining.min(RETRY_DELAY))
        .unwrap_or(RETRY_DELAY);

    tokio::select! {
        () = tokio::time::sleep(delay) => Ok(()),
        () = cancellation.cancelled() => Err(DownloadError::Cancelled),
    }
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), DownloadError> {
    if cancellation.is_cancelled() {
        Err(DownloadError::Cancelled)
    } else {
        Ok(())
    }
}

impl DownloadError {
    fn is_retryable(&self) -> bool {
        match self {
            DownloadError::Request { source, .. } => source.is_timeout() || source.is_connect(),
            DownloadError::Read { .. } => true,
            DownloadError::Status { status, .. } => status.is_server_error(),
            DownloadError::Client(_)
            | DownloadError::ResponseTooLarge { .. }
            | DownloadError::TimedOut { .. }
            | DownloadError::Cancelled
            | DownloadError::InvalidUtf8(_)
            | DownloadError::Runtime(_)
            // Повторять нечего: адрес тот же, и второй раз он безопаснее не станет.
            | DownloadError::InsecureScheme { .. }
            | DownloadError::UnusableUrl { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    /// Разбор отделён от правила: опечатка в адресе теста должна падать здесь,
    /// а не выглядеть как отказ по существу.
    fn transport_verdict(url: &str) -> Result<(), DownloadError> {
        let url = Url::parse(url).expect("a well-formed test address");
        ensure_transport_is_protected(&url)
    }

    /// Поднимает петлевой сервер, отвечающий заготовками по очереди.
    ///
    /// Чужое имя в `Location` до сети не доходит: правило проверяется на переходе,
    /// до того как адрес пойдёт в резолвер, — поэтому тест герметичен.
    ///
    /// Каждая заготовка закрывает соединение: иначе клиент отправил бы следующий
    /// запрос в то же самое, а сервер уже ждёт новое, и переход разваливается.
    fn serve(responses: Vec<&'static str>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the loopback server");
        let address = listener.local_addr().expect("local address of the server");
        let handle = std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        (format!("http://{address}/start"), handle)
    }

    fn fetch(url: &str) -> Result<Vec<u8>, DownloadError> {
        get_bytes(
            url,
            Some(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
    }

    #[test]
    fn plain_http_is_allowed_only_where_the_address_really_is_the_loopback() {
        for url in [
            "https://example.com/tool.zip",
            "http://127.0.0.1:3000/tool.zip",
            "http://localhost:3000/tool.zip",
            "http://[::1]:3000/tool.zip",
        ] {
            transport_verdict(url).unwrap_or_else(|error| panic!("{url} is allowed: {error}"));
        }
    }

    #[test]
    fn an_address_that_only_looks_like_the_loopback_is_refused() {
        // Каждая запись начинается как петлевой адрес и ведёт наружу. Последняя —
        // самая тихая: `127.0.0.1` здесь userinfo, а соединение идёт на `evil.com`.
        for url in [
            "http://127.evil.com/tool.zip",
            "http://127.0.0.1.nip.io/tool.zip",
            "http://127.0.0.1@evil.com/tool.zip",
            "http://example.com/tool.zip",
            "ftp://127.0.0.1/tool.zip",
        ] {
            let error = transport_verdict(url).expect_err("{url} is refused");
            assert!(
                matches!(&error, DownloadError::InsecureScheme { .. }),
                "{url} is refused as insecure, got {error:?}"
            );
        }
    }

    #[test]
    fn a_redirect_that_leaves_tls_is_refused_at_the_hop() {
        // Ровно тот сценарий, ради которого правило проверяется на каждом переходе:
        // адрес отвечает редиректом на имя, которое лишь начинается как петля.
        let (url, server) = serve(vec![concat!(
            "HTTP/1.1 302 Found\r\n",
            "Location: http://127.evil.com/payload.cfe\r\n",
            "Connection: close\r\n",
            "Content-Length: 0\r\n\r\n"
        )]);

        let error = fetch(&url).expect_err("a redirect off TLS is refused");
        server.join().expect("the server thread finishes");

        assert!(
            matches!(&error, DownloadError::Request { .. }),
            "a refused hop is reported as a failed request, got {error:?}"
        );
        assert!(!error.is_retryable(), "a refused hop is not retried");
    }

    /// Сервер отдал заголовки и замолчал посреди тела.
    ///
    /// По часам это неотличимо от медленной загрузки, по тишине — отличимо: байты
    /// перестали приходить. Без этого предела молчащее зеркало держало бы команду
    /// ровно столько, сколько та согласна ждать, а без срока на команду — навсегда.
    #[test]
    fn a_mirror_that_goes_silent_mid_body_is_not_waited_for_forever() {
        READ_IDLE_OVERRIDE.with(|value| value.set(Some(Duration::from_millis(150))));

        // Соединение не закрывается: закрытие — это конец потока, а проверяется молчание.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the silent server");
        let address = listener.local_addr().expect("local address");
        let server = std::thread::spawn(move || {
            // По одному соединению на каждую попытку: чтение повторяется.
            for _ in 0..RETRY_ATTEMPTS {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Length: 1024\r\n\r\n",
                        "only these bytes arrive"
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        let url = format!("http://{address}/start");

        let started = Instant::now();
        let error = fetch(&url).expect_err("a silent mirror does not finish the body");
        let waited = started.elapsed();
        drop(server);

        assert!(
            matches!(&error, DownloadError::Read { .. }),
            "silence is reported as a failed read, got {error:?}"
        );
        assert!(
            waited < Duration::from_secs(5),
            "the wait ends on silence, not on the command budget: waited {waited:?}"
        );
        READ_IDLE_OVERRIDE.with(|value| value.set(None));
    }

    #[test]
    fn a_redirect_that_stays_on_the_loopback_is_followed() {
        let (url, server) = serve(vec![
            concat!(
                "HTTP/1.1 302 Found\r\n",
                "Location: /next\r\n",
                "Connection: close\r\n",
                "Content-Length: 0\r\n\r\n"
            ),
            concat!(
                "HTTP/1.1 200 OK\r\n",
                "Connection: close\r\n",
                "Content-Length: 5\r\n\r\n",
                "HELLO"
            ),
        ]);

        let bytes = fetch(&url).expect("a hop that stays on the loopback is followed");
        server.join().expect("the server thread finishes");

        assert_eq!(bytes, b"HELLO");
    }

    #[test]
    fn an_address_that_is_not_an_address_is_named_as_such() {
        let error = fetch("example.com/tool.zip").expect_err("a bare name is not an address");

        assert!(
            matches!(&error, DownloadError::UnusableUrl { url } if url == "example.com/tool.zip"),
            "got {error:?}"
        );
        assert!(!error.is_retryable(), "a malformed address is not retried");
    }

    #[test]
    fn download_size_limit_accepts_boundary_size() {
        ensure_allowed_download_size("https://example.invalid/file", MAX_DOWNLOAD_BYTES)
            .expect("boundary size is accepted");
    }

    #[test]
    fn download_size_limit_rejects_oversized_response() {
        let error =
            ensure_allowed_download_size("https://example.invalid/file", MAX_DOWNLOAD_BYTES + 1)
                .expect_err("oversized response is rejected");

        assert!(matches!(
            error,
            DownloadError::ResponseTooLarge {
                size_bytes,
                max_bytes,
                ..
            } if size_bytes == MAX_DOWNLOAD_BYTES + 1 && max_bytes == MAX_DOWNLOAD_BYTES
        ));
    }
}
