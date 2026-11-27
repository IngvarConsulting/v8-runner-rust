use std::net::Ipv4Addr;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use hyper::{client::conn::http1, Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout_at, Instant};

use super::{is_rdbg_refusal, RDBG_PATH, RDBG_TIMEOUT};
use crate::platform::service;

pub(super) async fn owned_rdbg_answers(pid: u32, port: u16) -> Option<bool> {
    let deadline = Instant::now() + RDBG_TIMEOUT;
    let Ok(Ok(stream)) =
        timeout_at(deadline, TcpStream::connect((Ipv4Addr::LOCALHOST, port))).await
    else {
        return Some(false);
    };
    let client = stream.local_addr().ok()?;
    if client.ip() != Ipv4Addr::LOCALHOST {
        return None;
    }
    let client_port = client.port();
    #[cfg(target_os = "linux")]
    let peer_generation = || service::debug_connection_generation(pid, port, client_port);
    #[cfg(target_os = "macos")]
    let peer_generation = || {
        Ok::<_, std::io::Error>(service::macos_debug_connection_generation(
            pid,
            port,
            client_port,
        ))
    };
    // connect() can finish before dbgs has accepted the connection. Wait for
    // the reverse tuple to appear in its descriptor table before sending HTTP.
    let generation = timeout_at(deadline, async {
        loop {
            match peer_generation() {
                Ok(Some(generation)) => break Some(generation),
                Ok(None) => sleep(Duration::from_millis(10)).await,
                Err(_) => break None,
            }
        }
    })
    .await
    .ok()??;
    if Instant::now() >= deadline {
        return None;
    }

    let (mut request, connection) =
        timeout_at(deadline, http1::handshake::<_, Body>(TokioIo::new(stream)))
            .await
            .ok()?
            .ok()?;
    let driver = tokio::spawn(connection);
    let answers = timeout_at(deadline, async {
        let request_body = Request::post(RDBG_PATH)
            .header("Host", format!("{}:{port}", Ipv4Addr::LOCALHOST))
            .header("Content-Type", "application/xml")
            .header("Accept", "application/json")
            .header("Content-Length", "0")
            .body(Body::empty())
            .ok()?;
        let response = request.send_request(request_body).await.ok()?;
        if response.status() != StatusCode::BAD_REQUEST {
            return Some(false);
        }
        let body = to_bytes(Body::new(response.into_body()), 64 * 1024)
            .await
            .ok()?;
        Some(std::str::from_utf8(&body).is_ok_and(is_rdbg_refusal))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false);

    let proof = if !answers {
        Some(false)
    } else {
        (peer_generation().ok() == Some(Some(generation)) && Instant::now() < deadline)
            .then_some(true)
    };
    driver.abort();
    proof
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn owned_rdbg_requires_the_same_accepted_connection_after_the_reply() {
        for stays_open in [true, false] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (peer, _) = listener.accept().await.unwrap();
                let mut peer = BufReader::new(peer);
                let mut line = String::new();
                peer.read_line(&mut line).await.unwrap();
                assert_eq!(line.trim_end(), format!("POST {RDBG_PATH} HTTP/1.1"));
                loop {
                    line.clear();
                    assert_ne!(peer.read_line(&mut line).await.unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                peer.get_mut()
                    .write_all(
                        b"HTTP/1.1 400 Bad Request\r\nContent-Length: 33\r\n\r\nvirtual-resource-system Exception",
                    )
                    .await
                    .unwrap();
                if stays_open {
                    // The server retains the accepted FD while the client verifies it.
                    peer.read_line(&mut line).await.unwrap();
                }
            });
            assert_eq!(
                owned_rdbg_answers(std::process::id(), port).await,
                stays_open.then_some(true),
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn owned_rdbg_does_not_send_a_request_to_an_unattributed_connection() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (peer, _) = listener.accept().await.unwrap();
            let mut first_line = String::new();
            assert_eq!(
                BufReader::new(peer)
                    .read_line(&mut first_line)
                    .await
                    .unwrap(),
                0
            );
        });
        assert_eq!(owned_rdbg_answers(1, port).await, None);
        server.await.unwrap();
    }
}
