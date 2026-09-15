//! Минимальный клиент SFTP v3 поверх канала SSH: ровно те запросы, которыми раннер
//! обменивается файлами с точкой входа.
//!
//! Своя реализация, а не готовая библиотека, по одной причине: дескрипторы файлов у
//! шлюза `ibsrv` 8.3.27 — произвольные байты (замер 15.09.2026), а библиотечные клиенты
//! держат их строкой, портят, получают от шлюза код состояния 9 (`INVALID_HANDLE` из
//! четвёртой версии протокола) и ломаются на его разборе. Здесь дескриптор — байты, а
//! неизвестный код состояния — типизированный отказ с самим кодом, не паника.

use std::fmt;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const FXP_INIT: u8 = 1;
const FXP_VERSION: u8 = 2;
const FXP_OPEN: u8 = 3;
const FXP_CLOSE: u8 = 4;
const FXP_READ: u8 = 5;
const FXP_WRITE: u8 = 6;
const FXP_OPENDIR: u8 = 11;
const FXP_READDIR: u8 = 12;
const FXP_REMOVE: u8 = 13;
const FXP_MKDIR: u8 = 14;
const FXP_RMDIR: u8 = 15;
const FXP_STATUS: u8 = 101;
const FXP_HANDLE: u8 = 102;
const FXP_DATA: u8 = 103;
const FXP_NAME: u8 = 104;

const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;

const ATTR_SIZE: u32 = 0x0000_0001;
const ATTR_UIDGID: u32 = 0x0000_0002;
const ATTR_PERMISSIONS: u32 = 0x0000_0004;
const ATTR_ACMODTIME: u32 = 0x0000_0008;
const ATTR_EXTENDED: u32 = 0x8000_0000;

/// Флаги `SSH_FXP_OPEN`.
pub const OPEN_READ: u32 = 0x01;
pub const OPEN_WRITE: u32 = 0x02;
pub const OPEN_CREATE: u32 = 0x08;
pub const OPEN_TRUNCATE: u32 = 0x10;

/// Кусок чтения и записи: с запасом меньше окна канала SSH.
const CHUNK: u32 = 32 * 1024;

#[derive(Debug)]
pub enum SftpError {
    /// Точка входа ответила состоянием: код протокола и её текст дословно.
    Status { code: u32, message: String },
    /// Канал оборвался или ответ не разобрать.
    Io(String),
}

impl fmt::Display for SftpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status { code, message } if message.is_empty() => {
                write!(f, "sftp status {code}")
            }
            Self::Status { code, message } => write!(f, "sftp status {code}: {message}"),
            Self::Io(detail) => write!(f, "sftp channel: {detail}"),
        }
    }
}

impl SftpError {
    /// Обрыв канала, а не ответ точки входа: после него подсистему открывают заново.
    pub fn is_channel_loss(&self) -> bool {
        matches!(self, Self::Io(_))
    }
}

/// Запись каталога: имя и признак каталога из прав доступа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

pub struct SftpClient<S> {
    stream: S,
    next_id: u32,
}

impl<S: AsyncRead + AsyncWrite + Unpin> SftpClient<S> {
    /// `SSH_FXP_INIT` третьей версии; ответ — `SSH_FXP_VERSION`.
    pub async fn init(mut stream: S) -> Result<Self, SftpError> {
        send(&mut stream, FXP_INIT, &3u32.to_be_bytes()).await?;
        let (kind, _) = recv(&mut stream).await?;
        if kind != FXP_VERSION {
            return Err(SftpError::Io(format!(
                "expected SSH_FXP_VERSION, got packet type {kind}"
            )));
        }
        Ok(Self { stream, next_id: 0 })
    }

    async fn request(&mut self, kind: u8, body: &[u8]) -> Result<(u8, Vec<u8>), SftpError> {
        self.next_id = self.next_id.wrapping_add(1);
        let mut payload = self.next_id.to_be_bytes().to_vec();
        payload.extend_from_slice(body);
        send(&mut self.stream, kind, &payload).await?;
        loop {
            let (reply, data) = recv(&mut self.stream).await?;
            if data.len() < 4 {
                return Err(SftpError::Io(format!("reply {reply} without a request id")));
            }
            let id = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            // Ответы на запросы, которых уже никто не ждёт, пропускаются.
            if id == self.next_id {
                return Ok((reply, data[4..].to_vec()));
            }
        }
    }

    /// Итог запроса, отвечающего состоянием: `OK` — успех, остальное — отказ с кодом.
    async fn status_request(&mut self, kind: u8, body: &[u8]) -> Result<(), SftpError> {
        let (reply, data) = self.request(kind, body).await?;
        match parse_status(reply, &data)? {
            FX_OK => Ok(()),
            code => Err(SftpError::Status {
                code,
                message: status_message(&data),
            }),
        }
    }

    async fn handle_request(&mut self, kind: u8, body: &[u8]) -> Result<Vec<u8>, SftpError> {
        let (reply, data) = self.request(kind, body).await?;
        if reply == FXP_HANDLE {
            return Ok(read_string(&data, 0)?.0);
        }
        Err(status_error(reply, &data)?)
    }

    pub async fn mkdir(&mut self, path: &str) -> Result<(), SftpError> {
        let mut body = string(path.as_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        self.status_request(FXP_MKDIR, &body).await
    }

    pub async fn rmdir(&mut self, path: &str) -> Result<(), SftpError> {
        self.status_request(FXP_RMDIR, &string(path.as_bytes()))
            .await
    }

    pub async fn remove(&mut self, path: &str) -> Result<(), SftpError> {
        self.status_request(FXP_REMOVE, &string(path.as_bytes()))
            .await
    }

    pub async fn close(&mut self, handle: &[u8]) -> Result<(), SftpError> {
        self.status_request(FXP_CLOSE, &string(handle)).await
    }

    pub async fn open(&mut self, path: &str, flags: u32) -> Result<Vec<u8>, SftpError> {
        let mut body = string(path.as_bytes());
        body.extend_from_slice(&flags.to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        self.handle_request(FXP_OPEN, &body).await
    }

    /// Файл целиком: чтение кусками до `EOF`.
    pub async fn read_file(&mut self, path: &str) -> Result<Vec<u8>, SftpError> {
        let handle = self.open(path, OPEN_READ).await?;
        let mut contents = Vec::new();
        let outcome = async {
            loop {
                let mut body = string(&handle);
                body.extend_from_slice(&(contents.len() as u64).to_be_bytes());
                body.extend_from_slice(&CHUNK.to_be_bytes());
                let (reply, data) = self.request(FXP_READ, &body).await?;
                match reply {
                    FXP_DATA => {
                        let (chunk, _) = read_string(&data, 0)?;
                        if chunk.is_empty() {
                            return Ok(());
                        }
                        contents.extend_from_slice(&chunk);
                    }
                    _ => match parse_status(reply, &data)? {
                        FX_EOF => return Ok(()),
                        code => {
                            return Err(SftpError::Status {
                                code,
                                message: status_message(&data),
                            })
                        }
                    },
                }
            }
        }
        .await;
        let closed = self.close(&handle).await;
        outcome.and(closed).map(|()| contents)
    }

    /// Файл целиком: открыть с флагами, записать кусками, закрыть.
    pub async fn write_file(
        &mut self,
        path: &str,
        flags: u32,
        data: &[u8],
    ) -> Result<(), SftpError> {
        let handle = self.open(path, flags).await?;
        let outcome = async {
            for (index, chunk) in data.chunks(CHUNK as usize).enumerate() {
                let mut body = string(&handle);
                body.extend_from_slice(&((index * CHUNK as usize) as u64).to_be_bytes());
                body.extend_from_slice(&string(chunk));
                self.status_request(FXP_WRITE, &body).await?;
            }
            Ok(())
        }
        .await;
        let closed = self.close(&handle).await;
        outcome.and(closed)
    }

    /// Записи каталога без `.` и `..`.
    pub async fn list_dir(&mut self, path: &str) -> Result<Vec<DirEntry>, SftpError> {
        let handle = self
            .handle_request(FXP_OPENDIR, &string(path.as_bytes()))
            .await?;
        let mut entries = Vec::new();
        let outcome = async {
            loop {
                let (reply, data) = self.request(FXP_READDIR, &string(&handle)).await?;
                if reply != FXP_NAME {
                    match parse_status(reply, &data)? {
                        FX_EOF => return Ok(()),
                        code => {
                            return Err(SftpError::Status {
                                code,
                                message: status_message(&data),
                            })
                        }
                    }
                }
                let count = read_u32(&data, 0)?;
                let mut offset = 4;
                for _ in 0..count {
                    let (name, next) = read_string(&data, offset)?;
                    let (_longname, next) = read_string(&data, next)?;
                    let (is_dir, next) = read_attrs_is_dir(&data, next)?;
                    offset = next;
                    let name = String::from_utf8_lossy(&name).into_owned();
                    if name != "." && name != ".." {
                        entries.push(DirEntry { name, is_dir });
                    }
                }
            }
        }
        .await;
        let closed = self.close(&handle).await;
        outcome.and(closed).map(|()| entries)
    }
}

async fn send<S: AsyncWrite + Unpin>(
    stream: &mut S,
    kind: u8,
    body: &[u8],
) -> Result<(), SftpError> {
    let mut packet = ((body.len() + 1) as u32).to_be_bytes().to_vec();
    packet.push(kind);
    packet.extend_from_slice(body);
    stream
        .write_all(&packet)
        .await
        .map_err(|error| SftpError::Io(error.to_string()))
}

async fn recv<S: AsyncRead + Unpin>(stream: &mut S) -> Result<(u8, Vec<u8>), SftpError> {
    let mut length = [0u8; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(|error| SftpError::Io(error.to_string()))?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 {
        return Err(SftpError::Io("empty packet".to_owned()));
    }
    let mut body = vec![0u8; length];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|error| SftpError::Io(error.to_string()))?;
    let kind = body[0];
    body.remove(0);
    Ok((kind, body))
}

fn string(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(bytes);
    out
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, SftpError> {
    data.get(offset..offset + 4)
        .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| SftpError::Io("truncated packet".to_owned()))
}

fn read_string(data: &[u8], offset: usize) -> Result<(Vec<u8>, usize), SftpError> {
    let length = read_u32(data, offset)? as usize;
    let start = offset + 4;
    data.get(start..start + length)
        .map(|bytes| (bytes.to_vec(), start + length))
        .ok_or_else(|| SftpError::Io("truncated string".to_owned()))
}

/// Из атрибутов нужен один факт — каталог ли это; остальное пропускается по флагам.
fn read_attrs_is_dir(data: &[u8], offset: usize) -> Result<(bool, usize), SftpError> {
    let flags = read_u32(data, offset)?;
    let mut next = offset + 4;
    if flags & ATTR_SIZE != 0 {
        next += 8;
    }
    if flags & ATTR_UIDGID != 0 {
        next += 8;
    }
    let mut is_dir = false;
    if flags & ATTR_PERMISSIONS != 0 {
        let permissions = read_u32(data, next)?;
        is_dir = permissions & 0o170000 == 0o040000;
        next += 4;
    }
    if flags & ATTR_ACMODTIME != 0 {
        next += 8;
    }
    if flags & ATTR_EXTENDED != 0 {
        let count = read_u32(data, next)?;
        next += 4;
        for _ in 0..count {
            let (_, after_type) = read_string(data, next)?;
            let (_, after_data) = read_string(data, after_type)?;
            next = after_data;
        }
    }
    Ok((is_dir, next))
}

fn parse_status(reply: u8, data: &[u8]) -> Result<u32, SftpError> {
    if reply != FXP_STATUS {
        return Err(SftpError::Io(format!("unexpected packet type {reply}")));
    }
    read_u32(data, 0)
}

fn status_message(data: &[u8]) -> String {
    read_string(data, 4)
        .map(|(bytes, _)| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn status_error(reply: u8, data: &[u8]) -> Result<SftpError, SftpError> {
    let code = parse_status(reply, data)?;
    Ok(SftpError::Status {
        code,
        message: status_message(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Атрибуты записи каталога: права доступа говорят, каталог ли это, и остальные
    /// поля пропускаются по флагам, включая расширенные пары.
    #[test]
    fn dir_attrs_are_read_by_flags() {
        let mut data = (ATTR_SIZE | ATTR_PERMISSIONS | ATTR_EXTENDED)
            .to_be_bytes()
            .to_vec();
        data.extend_from_slice(&7u64.to_be_bytes());
        data.extend_from_slice(&0o040755u32.to_be_bytes());
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend(string(b"ext@example"));
        data.extend(string(b"value"));
        let (is_dir, next) = read_attrs_is_dir(&data, 0).expect("attrs");
        assert!(is_dir);
        assert_eq!(next, data.len());
    }

    /// Код состояния вне третьей версии протокола (9 у шлюза `ibsrv`) — отказ с кодом,
    /// а не сбой разбора.
    #[test]
    fn an_unknown_status_code_is_a_typed_refusal() {
        let mut data = 9u32.to_be_bytes().to_vec();
        data.extend(string(b"invalid handle"));
        data.extend(string(b""));
        let error = status_error(FXP_STATUS, &data).expect("status");
        assert_eq!(
            error.to_string(),
            "sftp status 9: invalid handle".to_owned()
        );
        assert!(!error.is_channel_loss());
    }
}
