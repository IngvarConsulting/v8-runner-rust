//! Поддельный сервер отладки 1С для `tests/cli_debug_server.rs`: тест собирает его `rustc` и кладёт
//! рядом с поддельным тонким клиентом. Он ведёт себя как `dbgs` в том, на что опирается раннер
//! (замер «Сервер отладки `dbgs`»): слушает петлю на `--port` или первом свободном порту
//! `--portRange`, пишет адрес в файл `--notify` в UTF-16 LE с BOM и отвечает на
//! `POST /e1crdbg/rdbg` с `Content-Type: application/xml` отказом 400 `virtual-resource-system`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let key = |name: &str| args.iter().find_map(|arg| arg.strip_prefix(name));
    let notify = key("--notify=").expect("--notify");
    let ports: Vec<u16> = match (key("--port="), key("--portRange=")) {
        (Some(port), _) => vec![port.parse().expect("--port")],
        (None, Some(range)) => {
            let mut bounds = range.split(':').map(|bound| bound.parse::<u16>().expect("range"));
            let (first, last) = (bounds.next().expect("first"), bounds.next().expect("last"));
            (first..=last).collect()
        }
        (None, None) => panic!("no port"),
    };
    let Some(listener) = ports
        .iter()
        .find_map(|port| TcpListener::bind(("127.0.0.1", *port)).ok())
    else {
        eprintln!("port taken");
        std::process::exit(1);
    };
    let port = listener.local_addr().expect("address").port();
    let mut address = vec![0xFF, 0xFE];
    for unit in format!("127.0.0.1:{port}").encode_utf16() {
        address.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(notify, address).expect("notify");
    println!("fake dbgs started on {port}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let mut stream = BufReader::new(&mut stream);
        let mut request = String::new();
        loop {
            let previous = request.len();
            match stream.read_line(&mut request) {
                Ok(0) | Err(_) => break,
                Ok(_) if matches!(&request[previous..], "\r\n" | "\n") => break,
                Ok(_) => {}
            }
        }
        if request.is_empty() {
            continue;
        }
        let request = request.to_ascii_lowercase();
        let rdbg = request.starts_with("post /e1crdbg/rdbg")
            && request.contains("content-type: application/xml");
        let (status, body) = if rdbg {
            (
                "400 Bad request",
                "{\"#exception\":\"{http://v8.1c.ru/8.2/virtual-resource-system}Exception\"}",
            )
        } else {
            ("404 Not found", "")
        };
        let close = request.lines().any(|line| line == "connection: close");
        let connection = if close { "close" } else { "keep-alive" };
        let written = write!(
            stream.get_mut(),
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n{body}",
            body.len()
        );
        if written.is_ok() && !close {
            // A real dbgs keeps the accepted socket open while status checks its owner.
            let _ = stream
                .get_ref()
                .set_read_timeout(Some(std::time::Duration::from_secs(3)));
            let _ = stream.read_line(&mut String::new());
        }
    }
}
