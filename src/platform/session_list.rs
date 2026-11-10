//! Список сеансов из текста отказа платформы: `/Out` пакетного Конфигуратора и `message`
//! агента при «Ошибка исключительной блокировки информационной базы.».
//!
//! Текст ничего не решает: что базу держат сеансы, сценарий узнаёт по состоянию — по
//! `error-type` агента и по блокировке файла базы (`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`).
//! Отсюда приходит только поле `list` отказа. Разбор закрытый: строка после заголовка, которую
//! он не понял, и строка без компьютера, начала или приложения дают `None` целиком, а не
//! список без неё (`INV.PLATFORM.LISTING-OUTPUT-IS-PARSED-FAIL-CLOSED`). Замер — раздел
//! «Чужие сеансы файловой базы при обновлении конфигурации БД (#211)».

use crate::domain::sessions::{ApplicationCode, SessionEntry};

/// Что исполнитель обновления делает с чужими сеансами. Ключ у каждого свой: у Конфигуратора
/// `-SessionTerminate force` (сообщения у ключа нет), у `ibcmd` `--session-terminate=force`
/// с обязательным `--session-terminate-message` — без него `ibcmd` с 8.3.25 ждёт ввода — и явный
/// `--session-terminate=disable` без принуждения. Файловой базе ключ не помогает: сеанс он не
/// завершает ни у одного исполнителя (замер #211), и туда его не передают.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTerminate<'a> {
    /// Чужие сеансы не трогать.
    Keep,
    /// Завершить их; сообщение — тем, чьи сеансы завершаются.
    Force { message: &'a str },
}

/// Заголовки списка: файловая база (8.3.27, 8.5.1, по-русски и по-английски) и прямой шлюз
/// автономного сервера (8.3.27.2074, #179).
const HEADERS: [&str; 3] = [
    "Активны сеансы:",
    "Активные сеансы и соединения:",
    "Existing sessions:",
];

/// Названия приложений по-русски и по-английски. Замерены тонкий и толстый клиент; остальные
/// взяты из названий платформы и без замера дают `application: null` только при расхождении.
const APPLICATIONS: [(&str, ApplicationCode); 12] = [
    ("Тонкий клиент", ApplicationCode::ThinClient),
    ("Thin client", ApplicationCode::ThinClient),
    ("Толстый клиент", ApplicationCode::ThickClient),
    ("1C:Enterprise", ApplicationCode::ThickClient),
    ("Конфигуратор", ApplicationCode::Designer),
    ("Designer", ApplicationCode::Designer),
    ("Фоновое задание", ApplicationCode::BackgroundJob),
    ("Background job", ApplicationCode::BackgroundJob),
    ("Веб-клиент", ApplicationCode::WebClient),
    ("Web client", ApplicationCode::WebClient),
    ("COM-соединение", ApplicationCode::ComConnection),
    ("COM connection", ApplicationCode::ComConnection),
];

/// Сеансы, названные текстом отказа; `None` — заголовка нет, сеансов нет или хоть одна
/// строка не разобрана.
pub fn parse_session_list(text: &str) -> Option<Vec<SessionEntry>> {
    let mut lines = text.lines().map(str::trim);
    lines.by_ref().find(|line| HEADERS.contains(line))?;
    let entries = lines
        .filter(|line| !line.is_empty())
        .map(parse_line)
        .collect::<Option<Vec<_>>>()?;
    (!entries.is_empty()).then_some(entries)
}

fn parse_line(line: &str) -> Option<SessionEntry> {
    let mut computer = None;
    let mut started = None;
    let mut title = None;
    let mut id = None;
    for part in line.split(", ") {
        let (key, value) = part.split_once(": ")?;
        let value = value.trim();
        match key.trim().to_lowercase().as_str() {
            "компьютер" | "computer" => computer = Some(value.to_owned()),
            "сеанс" | "session" => id = Some(value.to_owned()),
            "начат" | "сеанс начат" | "started" | "session started" => {
                started = Some(parse_started(value)?);
            }
            "приложение" | "application" => title = Some(value.to_owned()),
            // Неизвестное поле разбор не ломает: закрыт состав обязательных полей.
            _ => {}
        }
    }
    let application_title = title.filter(|title| !title.is_empty())?;
    Some(SessionEntry {
        computer: computer.filter(|computer| !computer.is_empty())?,
        application: APPLICATIONS
            .iter()
            .find(|(name, _)| *name == application_title)
            .map(|(_, code)| *code),
        application_title,
        started: started?,
        user: None,
        id: id.filter(|id| !id.is_empty()),
    })
}

/// `11.11.2026 в 2:49:42` и `11/11/2026 at 2:50:00 AM` → `2026-11-11T02:49:42`.
fn parse_started(value: &str) -> Option<String> {
    let (date, time) = value
        .split_once(" в ")
        .or_else(|| value.split_once(" at "))?;
    let (day, month, year) = if let Some((day, rest)) = date.split_once('.') {
        let (month, year) = rest.split_once('.')?;
        (day, month, year)
    } else {
        let (month, rest) = date.split_once('/')?;
        let (day, year) = rest.split_once('/')?;
        (day, month, year)
    };
    let (clock, meridiem) = match time.split_once(' ') {
        Some((clock, meridiem)) => (clock, Some(meridiem)),
        None => (time, None),
    };
    let mut parts = clock.split(':');
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let hour = match meridiem {
        None => hour,
        Some(meridiem) if meridiem.eq_ignore_ascii_case("AM") => hour % 12,
        Some(meridiem) if meridiem.eq_ignore_ascii_case("PM") => hour % 12 + 12,
        Some(_) => return None,
    };
    let (day, month, year): (u32, u32, u32) =
        (day.parse().ok()?, month.parse().ok()?, year.parse().ok()?);
    let valid = (1..=31).contains(&day)
        && (1..=12).contains(&month)
        && year >= 1000
        && hour < 24
        && minute < 60
        && second < 60;
    valid.then(|| format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `/Out` пакетного Конфигуратора 8.3.27.2342 у файловой базы с тонким и толстым клиентом.
    pub(crate) const MEASURED_LIST_RU: &str = "Обработка структуры базы данных...\r\n\r\nОшибка исключительной блокировки информационной базы.\r\nАктивны сеансы:\r\nкомпьютер: w11p-igor, сеанс: 2, начат: 11.11.2026 в 2:49:42, приложение: Тонкий клиент\r\nкомпьютер: w11p-igor, сеанс: 1, начат: 11.11.2026 в 2:49:41, приложение: Толстый клиент\r\n";

    /// То же с `/L en`.
    pub(crate) const MEASURED_LIST_EN: &str = "Process database structure...\r\n\r\nFailed to get exclusive lock on infobase.\r\nExisting sessions:\r\nComputer: w11p-igor, session: 2, started: 11/11/2026 at 2:50:00 AM, application: Thin client\r\nComputer: w11p-igor, session: 1, started: 11/11/2026 at 2:49:59 AM, application: 1C:Enterprise\r\n";

    /// `message` агента 8.3.27.2342 у файловой базы с тонким клиентом.
    pub(crate) const MEASURED_AGENT_MESSAGE: &str = "Ошибка исключительной блокировки информационной базы.\nАктивны сеансы:\nкомпьютер: w11p-igor, сеанс: 1, начат: 11.11.2026 в 2:37:17, приложение: Тонкий клиент";

    /// `/Out` пакетного Конфигуратора 8.3.27.2074 через прямой шлюз автономного сервера (#179).
    const MEASURED_SERVER_LIST: &str = "Обработка структуры базы данных...\nОшибка исключительной блокировки информационной базы.\nАктивные сеансы и соединения:\nкомпьютер: host-a, сеанс начат: 06.10.2026 в 0:48:09, приложение: Тонкий клиент\n";

    #[test]
    fn parses_the_measured_russian_list() {
        let list = parse_session_list(MEASURED_LIST_RU).expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].computer, "w11p-igor");
        assert_eq!(list[0].id.as_deref(), Some("2"));
        assert_eq!(list[0].started, "2026-11-11T02:49:42");
        assert_eq!(list[0].application, Some(ApplicationCode::ThinClient));
        assert_eq!(list[0].application_title, "Тонкий клиент");
        assert_eq!(list[1].application, Some(ApplicationCode::ThickClient));
        assert!(list.iter().all(|entry| entry.user.is_none()));
    }

    #[test]
    fn parses_the_measured_english_list() {
        let list = parse_session_list(MEASURED_LIST_EN).expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].started, "2026-11-11T02:50:00");
        assert_eq!(list[0].application, Some(ApplicationCode::ThinClient));
        assert_eq!(list[1].application, Some(ApplicationCode::ThickClient));
        assert_eq!(list[1].application_title, "1C:Enterprise");
    }

    #[test]
    fn parses_the_agent_message_and_the_server_form() {
        let agent = parse_session_list(MEASURED_AGENT_MESSAGE).expect("agent list");
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].started, "2026-11-11T02:37:17");

        let server = parse_session_list(MEASURED_SERVER_LIST).expect("server list");
        assert_eq!(server[0].computer, "host-a");
        assert_eq!(server[0].id, None);
        assert_eq!(server[0].started, "2026-10-06T00:48:09");
    }

    #[test]
    fn an_unrecognised_line_drops_the_whole_list() {
        let odd = MEASURED_LIST_RU.replace(
            "компьютер: w11p-igor, сеанс: 1, начат: 11.11.2026 в 2:49:41, приложение: Толстый клиент",
            "что-то ещё",
        );
        assert_eq!(parse_session_list(&odd), None);
        let no_start = MEASURED_LIST_RU.replace("начат: 11.11.2026 в 2:49:41, ", "");
        assert_eq!(parse_session_list(&no_start), None);
        assert_eq!(
            parse_session_list("Ошибка исключительной блокировки информационной базы."),
            None
        );
    }

    #[test]
    fn an_unknown_application_keeps_its_title_without_a_code() {
        let other = MEASURED_LIST_RU.replace("Толстый клиент", "Новое приложение");
        let list = parse_session_list(&other).expect("list");
        assert_eq!(list[1].application, None);
        assert_eq!(list[1].application_title, "Новое приложение");
    }

    #[test]
    fn twelve_pm_and_twelve_am_are_noon_and_midnight() {
        assert_eq!(
            parse_started("11/11/2026 at 12:05:00 PM").as_deref(),
            Some("2026-11-11T12:05:00")
        );
        assert_eq!(
            parse_started("11/11/2026 at 12:05:00 AM").as_deref(),
            Some("2026-11-11T00:05:00")
        );
        assert_eq!(parse_started("31.13.2026 в 1:00:00"), None);
    }
}
