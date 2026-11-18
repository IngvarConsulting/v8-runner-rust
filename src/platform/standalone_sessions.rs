//! Управление сеансами `ibsrv` по закреплённому SSH-ключу.
//! Кадры SCOP v1 и тело запроса измерены на 8.3.27.2074.

use std::cell::Cell;
use std::io;
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::platform::agent::{
    connect_ssh, AgentEndpoint, AgentError, HostKeyExpectation, WaitPolicy,
};
use crate::support::error::{AppError, EnvironmentLayer};

const VERSION: &[u8] = b"VER 00000001\nFIN 00000000\n";
const CALL: &[u8] = b"MSG 000000010000\n";
const REPLY: &[u8] = b"MSG 00000001000800000001\n";
const END: &[u8] = b"FIN 00000001\n";
const BREAK: &[u8] = b"BRK 00000001\n";
const DATA_ARGUMENT: &[u8] = b"ARG 000000010004data\n";
const LIST: &[u8] = b"CMD 000000010011list-session-info\n";
const TERMINATE: &[u8] = b"CMD 000000010011terminate-session\n";
const TEXT_FORMAT: &[u8] = b"CAP 00000001000bformat=text\n";
const RESULT: &[u8] = b"ARG 000000010010result=integer:0\n";
const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;

pub enum SessionCall<'a> {
    List,
    Terminate {
        session: Uuid,
        message: Option<&'a str>,
    },
}

/// Срок и отмена действуют от рукопожатия до последнего кадра ответа.
pub fn execute(
    endpoint: &AgentEndpoint,
    expected: &HostKeyExpectation,
    user: &str,
    password: &str,
    call: SessionCall<'_>,
    policy: &WaitPolicy,
) -> Result<String, AppError> {
    if !matches!(expected, HostKeyExpectation::Pinned(_)) {
        return Err(AppError::Validation(
            "standalone SCOP requires a pinned host key".to_owned(),
        ));
    }
    let operation = match call {
        SessionCall::List => "list-session-info",
        SessionCall::Terminate { .. } => "terminate-session",
    };
    policy.refuse_if_cancelled(operation)?;
    let request = encode(&call)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| AppError::Runtime(error.to_string()))?;
    let delivered = Cell::new(false);
    let named = endpoint.to_string();
    let exchange = async {
        let mut connection = connect_ssh(endpoint, expected, None).await?;
        let auth = if user.is_empty() && password.is_empty() {
            connection.authenticate_none("").await
        } else {
            connection.authenticate_password(user, password).await
        };
        if !auth
            .map_err(|source| AgentError::Handshake {
                endpoint: named.clone(),
                source,
            })?
            .success()
        {
            return Err(AgentError::AuthenticationRejected {
                endpoint: named.clone(),
                user: user.to_owned(),
            }
            .into());
        }
        let mut channel = connection.channel_open_session().await.map_err(|error| {
            AppError::environment_at(
                EnvironmentLayer::AgentEndpoint,
                format!("cannot open the standalone SSH channel at {named}: {error}"),
            )
        })?;
        channel
            .request_subsystem(true, "scop")
            .await
            .map_err(|error| {
                AppError::environment_at(
                    EnvironmentLayer::AgentEndpoint,
                    format!("cannot request the SCOP subsystem at {named}: {error}"),
                )
            })?;
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::WindowAdjusted { .. }) => {}
                _ => {
                    return Err(AppError::environment_at(
                        EnvironmentLayer::AgentEndpoint,
                        format!(
                            "standalone SSH gateway at {named} did not accept the SCOP subsystem"
                        ),
                    ));
                }
            }
        }
        let mut stream = channel.into_stream();
        stream.write_all(VERSION).await.map_err(wire_error)?;
        expect(&mut stream, VERSION).await.map_err(wire_error)?;
        policy.refuse_if_cancelled(operation)?;
        // Отправка уже могла произойти, даже если запись или ответ оборвутся.
        delivered.set(true);
        policy.mark_work_given();
        stream.write_all(&request).await.map_err(wire_error)?;
        decode(&mut stream, call).await.map_err(wire_error)
    };
    let deadline = async {
        match policy.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
            None => std::future::pending::<()>().await,
        }
    };
    runtime.block_on(async {
        let started = Instant::now();
        tokio::select! {
            biased;
            _ = policy.cancellation.cancelled() => Err(AgentError::Cancelled {
                command: operation.to_owned(),
                delivered: delivered.get(),
            }.into()),
            _ = deadline => Err(AgentError::TimedOut {
                command: operation.to_owned(),
                timeout_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }.into()),
            reply = exchange => reply,
        }
    })
}

fn wire_error(error: io::Error) -> AppError {
    let message = format!("standalone SCOP exchange failed: {error}");
    if error.kind() == io::ErrorKind::InvalidData {
        AppError::InvalidOutput(message)
    } else {
        AppError::Platform(message)
    }
}

fn encode(call: &SessionCall<'_>) -> Result<Vec<u8>, AppError> {
    let mut request = CALL.to_vec();
    match call {
        SessionCall::List => {
            request.extend_from_slice(LIST);
            request.extend_from_slice(TEXT_FORMAT);
        }
        SessionCall::Terminate { session, message } => {
            request.extend_from_slice(TERMINATE);
            request.extend_from_slice(BREAK);
            request.extend_from_slice(DATA_ARGUMENT);
            let body = termination_body(*session, *message)?;
            request.extend(format!("VAL 00000001{:04x}", body.len()).bytes());
            request.extend(body);
            request.push(b'\n');
        }
    }
    request.extend_from_slice(BREAK);
    request.extend_from_slice(END);
    Ok(request)
}

// Шаблон сериализованного запроса из ibcmd. Здесь объявлены только замеренные
// изменяемые поля: размер записи, указатель сообщения и GUID сеанса.
const TERMINATION_RECORD: [u64; 23] = [
    0xdfdb_0000_0000_0017,
    0x0000_0001_0100_0000,
    3,
    0x2005_013c_65fe_c571,
    0,
    10,
    18,
    19,
    21,
    0,
    0x2004_00bb_94f2_a2b6,
    15,
    0,
    0,
    0,
    0x4009_0000_0000_0001,
    0,
    0,
    0x4009_0000_0000_0000,
    0x4080_0000_0000_0000,
    0x0000_0400_0000_0000,
    0x4080_0000_0000_0000,
    0x0000_0400_0000_0000,
];
const MESSAGE_POINTER: usize = 9;
const REQUEST_UUID: usize = 128;

fn termination_body(session: Uuid, message: Option<&str>) -> Result<Vec<u8>, AppError> {
    let mut record = TERMINATION_RECORD;
    let mut text_value = Vec::new();
    if let Some(message) = message.filter(|value| !value.is_empty()) {
        if message.contains('\0') || message.len() > usize::from(u16::MAX) {
            return Err(AppError::Validation(
                "invalid standalone session message".to_owned(),
            ));
        }
        let units: Vec<_> = message.encode_utf16().collect();
        let count = u64::try_from(units.len() + 1).expect("bounded by the input length");
        let kind = if units.iter().all(|unit| *unit <= 0xff) {
            0x43
        } else {
            0x41
        };
        text_value.extend((count | 0x11 << 48 | kind << 56).to_le_bytes());
        text_value.extend(units.into_iter().chain([0]).flat_map(u16::to_le_bytes));
        text_value.resize(text_value.len().next_multiple_of(8), 0);
        record[MESSAGE_POINTER] = 23;
    }
    let byte_len = size_of_val(&record) + text_value.len();
    if byte_len > usize::from(u16::MAX) {
        return Err(AppError::Validation(
            "standalone session message exceeds the SCOP frame size".to_owned(),
        ));
    }
    record[0] = 0xdfdb_0000_0000_0000 | (byte_len / 8) as u64;
    let mut body: Vec<_> = record.into_iter().flat_map(u64::to_le_bytes).collect();
    body[REQUEST_UUID..REQUEST_UUID + 16].copy_from_slice(&session.to_bytes_le());
    body.extend(text_value);
    Ok(body)
}

async fn expect(reader: &mut (impl AsyncRead + Unpin), expected: &[u8]) -> io::Result<()> {
    let mut bytes = vec![0; expected.len()];
    reader.read_exact(&mut bytes).await?;
    if bytes != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected SCOP frame",
        ));
    }
    Ok(())
}

async fn read_value(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    reader.read_exact(&mut length).await?;
    let length = std::str::from_utf8(&length)
        .ok()
        .filter(|text| text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .and_then(|text| u16::from_str_radix(text, 16).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SCOP frame length"))?;
    let mut body = vec![0; usize::from(length)];
    reader.read_exact(&mut body).await?;
    expect(reader, b"\n").await?;
    Ok(body)
}

async fn decode(
    reader: &mut (impl AsyncRead + Unpin),
    call: SessionCall<'_>,
) -> io::Result<String> {
    expect(reader, REPLY).await?;
    match call {
        SessionCall::List => {
            expect(reader, LIST).await?;
            expect(reader, BREAK).await?;
            expect(reader, RESULT).await?;
            expect(reader, BREAK).await?;
            let mut output = Vec::new();
            loop {
                let mut header = [0; 12];
                reader.read_exact(&mut header).await?;
                match &header {
                    b"FIN 00000001" => {
                        expect(reader, b"\n").await?;
                        return String::from_utf8(output)
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
                    }
                    b"OUT 00000001" => {
                        let body = read_value(reader).await?;
                        if output.len() + body.len() > MAX_LIST_BYTES {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "SCOP session list exceeds the output limit",
                            ));
                        }
                        output.extend(body);
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected SCOP list frame",
                        ))
                    }
                }
            }
        }
        SessionCall::Terminate { session, .. } => {
            expect(reader, TERMINATE).await?;
            expect(reader, BREAK).await?;
            expect(reader, DATA_ARGUMENT).await?;
            expect(reader, b"VAL 00000001").await?;
            let body = read_value(reader).await?;
            expect(reader, BREAK).await?;
            expect(reader, END).await?;
            if !termination_reply(&body, session) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected SCOP termination record",
                ));
            }
            // Даже корректный пустой ответ бывает для уже исчезнувшего UUID.
            // Исчезновение подтверждает вызывающий повторным списком.
            Ok(String::new())
        }
    }
}

fn termination_reply(body: &[u8], session: Uuid) -> bool {
    const RECORD_TYPE: [u8; 24] = [
        0xdb, 0xdf, 0, 0, 0, 1, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0x06, 0xcf, 0x85, 0x44, 0x54,
        0x00,
    ];
    const RECORD_ITEM: [u64; 5] = [
        5,
        0x4080_0000_0000_0001,
        0x0000_0200_0000_0000,
        8,
        0x200b_08d8_c906_0504,
    ];
    if body.len() < 40
        || !body.len().is_multiple_of(8)
        || body[..6] != ((body.len() / 8) as u64).to_le_bytes()[..6]
        || body[6..30] != RECORD_TYPE
        || body[30..32] != [1, 0x20]
    {
        return false;
    }
    match body.len() {
        40 => body[32..] == [0; 8],
        88.. => {
            body[32..72]
                .iter()
                .copied()
                .eq(RECORD_ITEM.into_iter().flat_map(u64::to_le_bytes))
                && body[72..88] == session.to_bytes_le()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: Uuid = Uuid::from_u128(0x10203040_5060_4070_80a0_a0b0c0d0e0f0);

    #[test]
    fn termination_request_matches_the_native_scop_capture() {
        let expected = concat!(
            "170000000000dbdf0000000101000000030000000000000071c5fe653c010520",
            "00000000000000000a0000000000000012000000000000001300000000000000",
            "15000000000000000000000000000000b6a2f294bb0004200f00000000000000",
            "0000000000000000000000000000000000000000000000000100000000000940",
            "403020106050704080a0a0b0c0d0e0f000000000000009400000000000008040",
            "000000000004000000000000000080400000000000040000",
        );
        let request = termination_body(SESSION, None).unwrap();
        let actual: String = request.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(actual, expected);
        assert_eq!(termination_body(SESSION, Some("")).unwrap(), request);

        let with_message = termination_body(SESSION, Some("Ж😀")).unwrap();
        assert_eq!(with_message.len(), 200);
        assert_eq!(&with_message[..8], &[25, 0, 0, 0, 0, 0, 0xdb, 0xdf]);
        assert_eq!(&with_message[72..80], &[23, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &with_message[184..],
            &[4, 0, 0, 0, 0, 0, 0x11, 0x41, 0x16, 0x04, 0x3d, 0xd8, 0, 0xde, 0, 0]
        );
        assert_eq!(
            &termination_body(SESSION, Some("©")).unwrap()[184..],
            &[2, 0, 0, 0, 0, 0, 0x11, 0x43, 0xa9, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(termination_body(SESSION, Some(&"a".repeat(33_000))).is_err());
    }

    #[tokio::test]
    async fn scop_list_requires_the_complete_success_transaction() {
        let reply = [
            REPLY,
            LIST,
            BREAK,
            RESULT,
            BREAK,
            b"OUT 000000010004test\n",
            END,
        ]
        .concat();
        assert_eq!(
            decode(&mut reply.as_slice(), SessionCall::List)
                .await
                .unwrap(),
            "test"
        );
        let missing_finish = &reply[..reply.len() - END.len()];
        assert!(decode(&mut &missing_finish[..], SessionCall::List)
            .await
            .is_err());
        let mut wrong_transaction = reply;
        wrong_transaction[11] = b'2';
        assert!(decode(&mut wrong_transaction.as_slice(), SessionCall::List)
            .await
            .is_err());
    }

    #[test]
    fn termination_reply_names_only_the_selected_uuid() {
        let prefix: [u64; 9] = [
            0xdfdb_0000_0000_004c,
            0x0000_0001_0100_0000,
            3,
            0x2001_0054_4485_cf06,
            5,
            0x4080_0000_0000_0001,
            0x0000_0200_0000_0000,
            8,
            0x200b_08d8_c906_0504,
        ];
        let mut body: Vec<_> = prefix.into_iter().flat_map(u64::to_le_bytes).collect();
        body.extend(SESSION.to_bytes_le());
        body.resize(608, 0);
        assert!(termination_reply(&body, SESSION));
        assert!(!termination_reply(&body, Uuid::nil()));
        body.pop();
        assert!(!termination_reply(&body, SESSION));
        let empty: Vec<_> = [
            0xdfdb_0000_0000_0005_u64,
            0x0000_0001_0100_0000,
            3,
            0x2001_0054_4485_cf06,
            0,
        ]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
        assert!(termination_reply(&empty, SESSION));
    }
}
