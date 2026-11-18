//! Журнал регистрации файловой базы в последовательном формате: что в нём появилось после
//! метки (`INV.PLATFORM.THE-EVENT-LOG-IS-READ-IN-THE-SEQUENTIAL-FORMAT`).
//!
//! Замер — раздел «Исход входа интерактивного клиента в журнале регистрации файловой базы
//! (#777)». Журнал лежит в `<база>/1Cv8Log`: словарь `1Cv8.lgf` (дописывается) и записи
//! `<период>.lgp`. Оба файла начинаются заголовком `1CV8LOG(ver 2.0)`, строкой с UUID и пустой
//! строкой, дальше — записи скобочного формата через запятую. Записи словаря:
//! `{1,<uuid>,"<пользователь>",N}`, `{2,"<компьютер>",N}`, `{3,"<приложение>",N}`,
//! `{4,"<событие>",N}`; прочие виды раннеру не нужны. Запись событий:
//! `{ГГГГММДДччммсс,<статус транзакции>,{<транзакция>},<пользователь>,<компьютер>,<приложение>,
//! <соединение>,<событие>,<важность>,"<комментарий>",<метаданные>,{<данные>},"<представление>",
//! <сервер>,<порт>,<доп. порт>,<сеанс>,…}`. Журнал в формате SQLite (`1Cv8.lgd`) раннер не читает.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

use crate::platform::brackets::{self, Value};

/// Каталог журнала в каталоге файловой базы.
pub const LOG_DIR: &str = "1Cv8Log";
const DICTIONARY_FILE: &str = "1Cv8.lgf";
const SQLITE_FILE: &str = "1Cv8.lgd";
const RECORDS_EXTENSION: &str = "lgp";
const HEADER: &str = "1CV8LOG(ver 2.0)";
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Префикс событий входа: `Authentication`, `AuthenticationError` и родственных.
pub const LOGIN_EVENT_PREFIX: &str = "_$Session$_.Authentication";
pub const AUTHENTICATION: &str = "_$Session$_.Authentication";
pub const AUTHENTICATION_ERROR: &str = "_$Session$_.AuthenticationError";
pub const SESSION_START: &str = "_$Session$_.Start";
pub const SESSION_FINISH: &str = "_$Session$_.Finish";
pub const ACCESS_DENIED: &str = "_$Access$_.AccessDenied";

/// Почему журнал не прочитан. Любая из причин значит одно: свидетельства журнала нет.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    #[error(
        "the infobase event log is kept in the SQLite format (1Cv8.lgd), which the runner does not read; switch the event log to the sequential format"
    )]
    Sqlite,
    #[error("the event log file {file} has an unknown header")]
    UnknownHeader { file: String },
    #[error("the event log file {file} became shorter than it was")]
    Truncated { file: String },
    #[error("the event log file {file} holds a record the runner cannot read")]
    Unreadable { file: String },
    #[error("an event log record refers to entry {index} of kind {kind} that the dictionary does not have")]
    UnknownReference { kind: u64, index: u64 },
    #[error("the event log could not be read: {0}")]
    Io(#[from] std::io::Error),
}

/// Событие журнала с названиями из словаря.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    /// Время записи, местное время машины базы, до секунды.
    pub at: NaiveDateTime,
    pub event: String,
    /// Пользователь базы; `None` — поле пусто (так у `AuthenticationError`).
    pub user: Option<String>,
    pub computer: String,
    /// Код приложения, как его пишет платформа: `1CV8C`, `1CV8`, `Designer`…
    pub application: String,
    pub connection: u64,
    pub session: u64,
    /// Попытка входа из данных записи, если данные — попытка знакомого вида.
    pub login: Option<LoginAttempt>,
}

/// Кто пытался войти — данные событий входа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginAttempt {
    /// `{"P",{518,{"S","<имя>"},{"S","<пользователь ОС>"},…}}`: вход по имени.
    ByName { user: String, os_user: String },
    /// `{"P",{513,{"S","<пользователь ОС>"},…}}`: вход по пользователю ОС.
    ByOsUser { os_user: String },
}

/// Чтение журнала после метки: где кончалось прочитанное в каждом файле.
#[derive(Debug)]
pub struct EventLogTail {
    dir: PathBuf,
    dictionary: HashMap<(u64, u64), String>,
    /// Смещение за последней прочитанной записью словаря; 0 — заголовок ещё не прочитан.
    dictionary_offset: u64,
    /// Файл записей и смещение за последней прочитанной полной записью. Файла, которого нет
    /// в перечне, при метке не было: он читается с начала.
    records: BTreeMap<OsString, u64>,
}

impl EventLogTail {
    /// Метка до запуска клиента: размеры файлов записей. Журнала ещё нет — всё, что в нём
    /// появится, новое. Журнал SQLite и словарь с незнакомым заголовком — отказ.
    pub fn mark(base_dir: &Path) -> Result<Self, EventLogError> {
        let dir = base_dir.join(LOG_DIR);
        refuse_sqlite(&dir)?;
        let mut records = BTreeMap::new();
        for (name, path) in record_files(&dir)? {
            records.insert(name, std::fs::metadata(path)?.len());
        }
        let mut tail = Self {
            dir,
            dictionary: HashMap::new(),
            dictionary_offset: 0,
            records,
        };
        tail.read_dictionary()?;
        Ok(tail)
    }

    /// События, записанные после метки и прошлых чтений. Запись, которую ещё дописывают,
    /// остаётся до следующего чтения.
    pub fn read_new(&mut self) -> Result<Vec<LogEvent>, EventLogError> {
        refuse_sqlite(&self.dir)?;
        self.read_dictionary()?;
        let mut events = Vec::new();
        for (name, path) in record_files(&self.dir)? {
            let offset = self.records.get(&name).copied().unwrap_or(0);
            let (end, read) = read_records(&path, offset)?;
            self.records.insert(name, end);
            for record in read {
                events.push(self.resolved(&record, &path)?);
            }
        }
        Ok(events)
    }

    fn read_dictionary(&mut self) -> Result<(), EventLogError> {
        let path = self.dir.join(DICTIONARY_FILE);
        if !path.exists() {
            return Ok(());
        }
        let (end, entries) = read_records(&path, self.dictionary_offset)?;
        self.dictionary_offset = end;
        for entry in entries {
            let Some(items) = entry.as_list() else {
                return Err(unreadable(&path));
            };
            let kind = items.first().and_then(Value::as_number);
            let named = match (kind, items) {
                (Some(1), [_, _, name, index, ..]) => Some((1, name, index)),
                (Some(kind @ 2..=4), [_, name, index, ..]) => Some((kind, name, index)),
                _ => None,
            };
            if let Some((kind, name, index)) = named {
                let (Some(name), Some(index)) = (name.as_text(), index.as_number()) else {
                    return Err(unreadable(&path));
                };
                self.dictionary.insert((kind, index), name.to_owned());
            }
        }
        Ok(())
    }

    /// Запись с названиями из словаря. Номер, которого в словаре нет, дочитывает словарь
    /// ещё раз: запись и словарь пишутся порознь.
    fn resolved(&mut self, record: &Value, path: &Path) -> Result<LogEvent, EventLogError> {
        match self.resolve(record, path) {
            Err(EventLogError::UnknownReference { .. }) => {
                self.read_dictionary()?;
                self.resolve(record, path)
            }
            other => other,
        }
    }

    fn resolve(&self, record: &Value, path: &Path) -> Result<LogEvent, EventLogError> {
        let fields = record
            .as_list()
            .filter(|fields| fields.len() >= 17)
            .ok_or_else(|| unreadable(path))?;
        let number = |at: usize| fields[at].as_number().ok_or_else(|| unreadable(path));
        let named = |kind: u64, at: usize| -> Result<Option<String>, EventLogError> {
            match number(at)? {
                0 => Ok(None),
                index => self
                    .dictionary
                    .get(&(kind, index))
                    .cloned()
                    .map(Some)
                    .ok_or(EventLogError::UnknownReference { kind, index }),
            }
        };
        let at = fields[0]
            .as_atom()
            .and_then(|stamp| NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M%S").ok())
            .ok_or_else(|| unreadable(path))?;
        Ok(LogEvent {
            at,
            user: named(1, 3)?,
            computer: named(2, 4)?.unwrap_or_default(),
            application: named(3, 5)?.unwrap_or_default(),
            connection: number(6)?,
            event: named(4, 7)?.unwrap_or_default(),
            session: number(16)?,
            login: login_attempt(&fields[11]),
        })
    }
}

/// Данные попытки входа: `{"P",{518|513,{"S",…},…}}`; иной вид — не попытка.
fn login_attempt(data: &Value) -> Option<LoginAttempt> {
    let [kind, body] = data.as_list()? else {
        return None;
    };
    if kind.as_text() != Some("P") {
        return None;
    }
    let body = body.as_list()?;
    let text = |at: usize| -> Option<String> {
        match body.get(at)?.as_list()? {
            [tag, value] if tag.as_text() == Some("S") => value.as_text().map(str::to_owned),
            _ => None,
        }
    };
    match body.first()?.as_number()? {
        518 => Some(LoginAttempt::ByName {
            user: text(1)?,
            os_user: text(2)?,
        }),
        513 => Some(LoginAttempt::ByOsUser { os_user: text(1)? }),
        _ => None,
    }
}

fn refuse_sqlite(dir: &Path) -> Result<(), EventLogError> {
    if dir.join(SQLITE_FILE).exists() {
        return Err(EventLogError::Sqlite);
    }
    Ok(())
}

/// Файлы записей `*.lgp` каталога журнала по имени; каталога нет — файлов нет.
fn record_files(dir: &Path) -> Result<Vec<(OsString, PathBuf)>, EventLogError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_records = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(RECORDS_EXTENSION));
        if is_records && path.is_file() {
            if let Some(name) = path.file_name() {
                files.push((name.to_owned(), path.clone()));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Полные записи файла после `offset` и смещение за последней из них. С начала файла сперва
/// читается заголовок; недописанный заголовок — ещё не записи.
fn read_records(path: &Path, offset: u64) -> Result<(u64, Vec<Value>), EventLogError> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length < offset {
        return Err(EventLogError::Truncated { file: shown(path) });
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut cursor = 0;
    if offset == 0 {
        match header_end(&bytes) {
            None => return Ok((0, Vec::new())),
            Some(Err(())) => return Err(EventLogError::UnknownHeader { file: shown(path) }),
            Some(Ok(end)) => cursor = end,
        }
    }
    let mut records = Vec::new();
    loop {
        match brackets::complete_record(&bytes[cursor..]) {
            None => break,
            Some(Err(_)) => return Err(unreadable(path)),
            Some(Ok(range)) => {
                let text = std::str::from_utf8(&bytes[cursor + range.start..cursor + range.end])
                    .map_err(|_| unreadable(path))?;
                records.push(brackets::parse(text).ok_or_else(|| unreadable(path))?);
                cursor += range.end;
            }
        }
    }
    let consumed = u64::try_from(cursor).map_err(|_| unreadable(path))?;
    Ok((offset + consumed, records))
}

/// Конец заголовка — за строкой с UUID. `None` — заголовок ещё дописывают, `Err` — он чужой.
fn header_end(bytes: &[u8]) -> Option<Result<usize, ()>> {
    let start = if bytes.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else if UTF8_BOM.starts_with(bytes) {
        return None;
    } else {
        0
    };
    let body = &bytes[start..];
    let Some(first_end) = body.iter().position(|&byte| byte == b'\n') else {
        let expected = format!("{HEADER}\r\n");
        return if expected.as_bytes().starts_with(body) {
            None
        } else {
            Some(Err(()))
        };
    };
    if body[..first_end].trim_ascii() != HEADER.as_bytes() {
        return Some(Err(()));
    }
    let after_first = first_end + 1;
    let uuid_end = body[after_first..].iter().position(|&byte| byte == b'\n')?;
    Some(Ok(start + after_first + uuid_end + 1))
}

fn unreadable(path: &Path) -> EventLogError {
    EventLogError::Unreadable { file: shown(path) }
}

fn shown(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Настоящие записи журнала из замера #777 (Windows, 8.3.27): словарь и записи с
    //! переводами строк CRLF и UTF-8 с BOM, как их пишет платформа.

    use std::io::Write;
    use std::path::Path;

    pub const LOG_HEADER: &str = "\u{feff}1CV8LOG(ver 2.0)\r\ndeb56b87-7626-4f40-bcb7-efc726fcc510\r\n\r\n";

    /// Словарь: пользователи `Админ` (2) и `Пробел Кавычка"` (3), компьютер `w11p-igor` (1),
    /// приложения `Designer` (1), `1CV8` (2), `1CV8C` (3).
    pub const DICTIONARY: &str = "{1,071523a4-516f-4fce-ba4b-0d11ab7a1893,\"\",1},\r\n{2,\"w11p-igor\",1},\r\n{3,\"Designer\",1},\r\n{4,\"_$Session$_.Authentication\",1},\r\n{13,1,1},\r\n{4,\"_$Session$_.Start\",2},\r\n{4,\"_$Session$_.Finish\",3},\r\n{3,\"1CV8\",2},\r\n{1,0bc1cbfa-8a5e-48cc-8c5f-c7d37209f115,\"Админ\",2},\r\n{13,1,2},\r\n{4,\"_$Session$_.AuthenticationError\",12},\r\n{1,3643c44a-3fe6-43e3-b2fd-7273eb612557,\"Пробел Кавычка\"\"\",3},\r\n{13,1,3},\r\n{4,\"_$Access$_.AccessDenied\",13},\r\n{3,\"1CV8C\",3}";

    /// `Authentication` толстого клиента пользователя `Админ`, соединение 1.
    pub const THICK_AUTHENTICATION: &str = "{20261119040014,N,\r\n{0,0},2,1,2,1,1,I,\"\",0,\r\n{\"P\",\r\n{518,\r\n{\"S\",\"Админ\"},\r\n{\"S\",\"w11p-igor\\Igor\"},\r\n{\"N\",1}\r\n}\r\n},\"\",0,0,0,1,0,\r\n{0}\r\n}";
    /// `Start` того же сеанса.
    pub const THICK_START: &str = "{20261119040014,N,\r\n{0,0},2,1,2,1,2,I,\"\",0,\r\n{\"U\"},\"\",0,0,0,1,0,\r\n{0}\r\n}";
    /// `Finish` того же сеанса.
    pub const THICK_FINISH: &str = "{20261119040019,N,\r\n{0,0},2,1,2,1,3,I,\"\",0,\r\n{\"U\"},\"\",0,0,0,1,0,\r\n{0}\r\n}";
    /// `AuthenticationError` толстого клиента: неверный пароль `Админ`.
    pub const THICK_AUTHENTICATION_ERROR: &str = "{20261119040916,N,\r\n{0,0},0,1,2,1,12,I,\"\",0,\r\n{\"P\",\r\n{518,\r\n{\"S\",\"Админ\"},\r\n{\"S\",\"w11p-igor\\Igor\"},\r\n{\"N\",1}\r\n}\r\n},\"\",0,0,0,1,0,\r\n{0}\r\n}";
    /// `AuthenticationError` толстого клиента без `/N`: попытка входа по ОС.
    pub const THICK_OS_AUTHENTICATION_ERROR: &str = "{20261119040931,N,\r\n{0,0},0,1,2,1,12,I,\"\",0,\r\n{\"P\",\r\n{513,\r\n{\"S\",\"w11p-igor\\Igor\"},\r\n{\"N\",0}\r\n}\r\n},\"\",0,0,0,1,0,\r\n{0}\r\n}";
    /// `AuthenticationError` тонкого клиента: неверный пароль `Админ`.
    pub const THIN_AUTHENTICATION_ERROR: &str = "{20261119035650,N,\r\n{0,0},0,1,3,1,12,I,\"\",0,\r\n{\"P\",\r\n{518,\r\n{\"S\",\"Админ\"},\r\n{\"S\",\"w11p-igor\\Igor\"},\r\n{\"N\",1}\r\n}\r\n},\"\",0,0,0,1,0,\r\n{0}\r\n}";

    /// Записи после первой — с ведущей запятой, как их дописывает платформа.
    pub fn joined(records: &[&str]) -> String {
        records
            .iter()
            .enumerate()
            .map(|(at, record)| {
                if at == 0 {
                    (*record).to_owned()
                } else {
                    format!(",\r\n{record}")
                }
            })
            .collect()
    }

    /// Журнал базы `base`: словарь и файл записей периода с заголовками.
    pub fn write_log(base: &Path, records: &[&str]) {
        let dir = base.join(super::LOG_DIR);
        std::fs::create_dir_all(&dir).expect("log dir");
        std::fs::write(
            dir.join(super::DICTIONARY_FILE),
            format!("{LOG_HEADER}{DICTIONARY}"),
        )
        .expect("dictionary");
        std::fs::write(
            dir.join("20261116000000.lgp"),
            format!("{LOG_HEADER}{}", joined(records)),
        )
        .expect("records");
    }

    /// Дописать записи в файл периода, как платформа: через запятую.
    pub fn append(base: &Path, file: &str, text: impl AsRef<[u8]>) {
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(base.join(super::LOG_DIR).join(file))
            .expect("open log");
        handle.write_all(text.as_ref()).expect("append");
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn at(stamp: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M%S").expect("stamp")
    }

    /// Обе формы попытки входа из замера: по имени (518) и по пользователю ОС (513); записи
    /// до метки не возвращаются.
    #[test]
    fn records_after_the_mark_are_read_with_their_login_attempts() {
        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[THICK_AUTHENTICATION, THICK_START]);
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        assert_eq!(tail.read_new().expect("nothing new"), Vec::new());

        append(
            base.path(),
            "20261116000000.lgp",
            &format!(
                ",\r\n{}",
                joined(&[THICK_AUTHENTICATION_ERROR, THICK_OS_AUTHENTICATION_ERROR])
            ),
        );
        let events = tail.read_new().expect("read");
        assert_eq!(
            events,
            vec![
                LogEvent {
                    at: at("20261119040916"),
                    event: AUTHENTICATION_ERROR.to_owned(),
                    user: None,
                    computer: "w11p-igor".to_owned(),
                    application: "1CV8".to_owned(),
                    connection: 1,
                    session: 1,
                    login: Some(LoginAttempt::ByName {
                        user: "Админ".to_owned(),
                        os_user: "w11p-igor\\Igor".to_owned(),
                    }),
                },
                LogEvent {
                    at: at("20261119040931"),
                    event: AUTHENTICATION_ERROR.to_owned(),
                    user: None,
                    computer: "w11p-igor".to_owned(),
                    application: "1CV8".to_owned(),
                    connection: 1,
                    session: 1,
                    login: Some(LoginAttempt::ByOsUser {
                        os_user: "w11p-igor\\Igor".to_owned(),
                    }),
                },
            ]
        );
        assert_eq!(tail.read_new().expect("read again"), Vec::new());
    }

    /// Запись, которую дописывают, ждёт следующего чтения; обрыв посреди буквы — не ошибка.
    /// Имя пользователя с удвоенной кавычкой и комментарий с переводом строки читаются.
    #[test]
    fn a_record_split_across_reads_is_read_once_it_is_complete() {
        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[THICK_AUTHENTICATION]);
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        let record = "{20261119040014,N,\r\n{0,0},3,1,2,7,2,I,\"две\r\nстроки \"\"в кавычках\"\"\",0,\r\n{\"U\"},\"\",0,0,0,4,0,\r\n{0}\r\n}";
        let appended = format!(",\r\n{record}");
        let split = appended.find("строки").expect("split") + 1;
        append(base.path(), "20261116000000.lgp", &appended.as_bytes()[..split]);
        assert_eq!(tail.read_new().expect("partial"), Vec::new());
        append(base.path(), "20261116000000.lgp", &appended.as_bytes()[split..]);
        let events = tail.read_new().expect("complete");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, SESSION_START);
        assert_eq!(events[0].user.as_deref(), Some("Пробел Кавычка\""));
        assert_eq!((events[0].connection, events[0].session), (7, 4));
        assert_eq!(events[0].login, None);
    }

    /// Новый файл периода и словарь, выросший после метки, читаются; журнала, которого при
    /// метке не было, — тоже с начала.
    #[test]
    fn a_new_period_file_and_a_grown_dictionary_are_read() {
        let base = tempfile::tempdir().expect("base");
        let mut absent = EventLogTail::mark(base.path()).expect("mark without a log");
        assert_eq!(absent.read_new().expect("no log"), Vec::new());
        write_log(base.path(), &[THICK_AUTHENTICATION]);
        assert_eq!(absent.read_new().expect("log appeared").len(), 1);

        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        append(
            base.path(),
            DICTIONARY_FILE,
            ",\r\n{1,11111111-2222-3333-4444-555555555555,\"Новый\",9},\r\n{4,\"_$Session$_.Start\",2}",
        );
        std::fs::write(
            base.path().join(LOG_DIR).join("20261120000000.lgp"),
            format!(
                "{LOG_HEADER}{}",
                THICK_START.replacen("{0,0},2,", "{0,0},9,", 1)
            ),
        )
        .expect("new period");
        let events = tail.read_new().expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user.as_deref(), Some("Новый"));
    }

    #[test]
    fn a_sqlite_log_is_refused_at_the_mark_and_later() {
        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[]);
        std::fs::write(base.path().join(LOG_DIR).join(SQLITE_FILE), b"SQLite format 3")
            .expect("lgd");
        assert!(matches!(
            EventLogTail::mark(base.path()),
            Err(EventLogError::Sqlite)
        ));

        let base = tempfile::tempdir().expect("base");
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        std::fs::create_dir_all(base.path().join(LOG_DIR)).expect("dir");
        std::fs::write(base.path().join(LOG_DIR).join(SQLITE_FILE), b"SQLite format 3")
            .expect("lgd");
        assert!(matches!(tail.read_new(), Err(EventLogError::Sqlite)));
    }

    #[test]
    fn a_shortened_file_an_unknown_header_and_an_unknown_entry_are_refused() {
        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[THICK_AUTHENTICATION, THICK_START]);
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        write_log(base.path(), &[THICK_AUTHENTICATION]);
        assert!(matches!(
            tail.read_new(),
            Err(EventLogError::Truncated { .. })
        ));

        let base = tempfile::tempdir().expect("base");
        let dir = base.path().join(LOG_DIR);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join(DICTIONARY_FILE), "1CV8LOG(ver 3.0)\r\nuuid\r\n\r\n")
            .expect("dictionary");
        assert!(matches!(
            EventLogTail::mark(base.path()),
            Err(EventLogError::UnknownHeader { .. })
        ));

        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[]);
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        append(
            base.path(),
            "20261116000000.lgp",
            &THICK_START.replacen("{0,0},2,", "{0,0},77,", 1),
        );
        assert!(matches!(
            tail.read_new(),
            Err(EventLogError::UnknownReference { kind: 1, index: 77 })
        ));

        let base = tempfile::tempdir().expect("base");
        write_log(base.path(), &[]);
        let mut tail = EventLogTail::mark(base.path()).expect("mark");
        append(base.path(), "20261116000000.lgp", "garbage{1}");
        assert!(matches!(
            tail.read_new(),
            Err(EventLogError::Unreadable { .. })
        ));
    }

    #[test]
    fn a_header_being_written_is_not_yet_records() {
        assert_eq!(header_end(b""), None);
        assert_eq!(header_end(b"\xEF\xBB"), None);
        assert_eq!(header_end(b"\xEF\xBB\xBF1CV8LOG(ver"), None);
        assert_eq!(header_end(b"\xEF\xBB\xBF1CV8LOG(ver 2.0)\r\nuuid"), None);
        assert_eq!(
            header_end(LOG_HEADER.as_bytes()),
            Some(Ok(LOG_HEADER.len() - "\r\n".len()))
        );
        assert_eq!(header_end(b"SQLite format 3\0"), Some(Err(())));
    }
}
