//! Скобочный формат 1С — единственный его читатель в раннере.
//!
//! Значение — список `{…}` со значениями через запятую, строка в кавычках или атом (число,
//! UUID, буква — всё, что до запятой или закрывающей скобки). В строке `""` — одна кавычка,
//! `\` — обычный символ, переводы строк бывают внутри. Пробелы и переводы строк между
//! значениями ничего не значат. Так записаны журнал регистрации (`1Cv8.lgf`, `*.lgp`) и
//! блокировка начала сеансов (`1Cv8.cdn`).

/// Значение скобочного формата.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `{…}`.
    List(Vec<Value>),
    /// Строка в кавычках, уже без удвоения кавычек.
    Text(String),
    /// Значение без кавычек, как оно написано.
    Atom(String),
}

impl Value {
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            Self::Text(_) | Self::Atom(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::List(_) | Self::Atom(_) => None,
        }
    }

    pub fn as_atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(atom),
            Self::List(_) | Self::Text(_) => None,
        }
    }

    /// Атом — неотрицательное десятичное число.
    pub fn as_number(&self) -> Option<u64> {
        self.as_atom()?.parse().ok()
    }
}

/// Строка в кавычках в начале `text`: её значение и сколько байт она заняла с кавычками.
/// `None` — `text` не начинается с кавычки или строка не закрыта.
pub fn quoted(text: &str) -> Option<(String, usize)> {
    let body = text.strip_prefix('"')?;
    let mut value = String::new();
    let mut chars = body.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        if ch != '"' {
            value.push(ch);
            continue;
        }
        if chars.peek().is_some_and(|&(_, next)| next == '"') {
            chars.next();
            value.push('"');
            continue;
        }
        return Some((value, 1 + at + 1));
    }
    None
}

/// Первая строка в кавычках, где бы она ни стояла.
pub fn first_quoted(text: &str) -> Option<String> {
    let start = text.find('"')?;
    quoted(&text[start..]).map(|(value, _)| value)
}

/// Конец первой полной записи `{…}` в `bytes`: смещение за её закрывающей скобкой. До записи
/// допустимы только пробелы, переводы строк и запятые. `None` — записи ещё нет целиком (её
/// дописывают), `Some(Err(at))` — на смещении `at` стоит то, чего между записями быть не может.
///
/// Байты, а не текст: так полнота записи видна до декодирования UTF-8, и обрыв посреди
/// многобайтной буквы не мешает — ни одна её часть не совпадает с `{`, `}` или `"`.
pub fn complete_record(bytes: &[u8]) -> Option<Result<std::ops::Range<usize>, usize>> {
    let start = match bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b','))?
    {
        at if bytes[at] == b'{' => at,
        at => return Some(Err(at)),
    };
    let mut depth = 0usize;
    let mut in_text = false;
    for (offset, byte) in bytes[start..].iter().enumerate() {
        match (in_text, byte) {
            // `""` внутри строки — выход и тут же вход: баланс тот же.
            (_, b'"') => in_text = !in_text,
            (false, b'{') => depth += 1,
            (false, b'}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(Ok(start..start + offset + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Разбор одного значения целиком; вокруг него допустимы только пробелы и переводы строк.
pub fn parse(text: &str) -> Option<Value> {
    let (value, used) = parse_value(text)?;
    text[used..].trim().is_empty().then_some(value)
}

fn skip_space(text: &str, at: usize) -> usize {
    at + (text[at..].len() - text[at..].trim_start().len())
}

/// Значение с начала `text` (после пробелов) и смещение за ним.
fn parse_value(text: &str) -> Option<(Value, usize)> {
    let at = skip_space(text, 0);
    let rest = &text[at..];
    match rest.chars().next()? {
        '{' => {
            let mut items = Vec::new();
            let mut cursor = skip_space(text, at + 1);
            if text[cursor..].starts_with('}') {
                return Some((Value::List(items), cursor + 1));
            }
            loop {
                let (item, used) = parse_value(&text[cursor..])?;
                items.push(item);
                cursor = skip_space(text, cursor + used);
                match text[cursor..].chars().next()? {
                    ',' => cursor += 1,
                    '}' => return Some((Value::List(items), cursor + 1)),
                    _ => return None,
                }
            }
        }
        '"' => {
            let (value, used) = quoted(rest)?;
            Some((Value::Text(value), at + used))
        }
        _ => {
            let end = rest.find([',', '}', '{', '"']).unwrap_or(rest.len());
            let atom = rest[..end].trim();
            if atom.is_empty() {
                return None;
            }
            Some((Value::Atom(atom.to_owned()), at + end))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn atom(value: &str) -> Value {
        Value::Atom(value.to_owned())
    }

    /// Запись словаря журнала из замера #777: имя пользователя с удвоенной кавычкой.
    #[test]
    fn a_record_keeps_doubled_quotes_backslashes_and_line_breaks() {
        let record = "{1,3643c44a-3fe6-43e3-b2fd-7273eb612557,\"Пробел Кавычка\"\"\",3}";
        assert_eq!(
            parse(record),
            Some(Value::List(vec![
                atom("1"),
                atom("3643c44a-3fe6-43e3-b2fd-7273eb612557"),
                text("Пробел Кавычка\""),
                atom("3"),
            ]))
        );
        let nested = "{\"P\",\r\n{518,\r\n{\"S\",\"\\\\NOSUCHDOMAIN\\nosuchuser\"},\r\n{\"S\",\"две\r\nстроки\"}\r\n}\r\n}";
        let value = parse(nested).expect("nested record");
        let data = value.as_list().expect("list")[1].as_list().expect("data");
        assert_eq!(data[0].as_number(), Some(518));
        assert_eq!(
            data[1].as_list().expect("os user")[1].as_text(),
            Some("\\\\NOSUCHDOMAIN\\nosuchuser")
        );
        assert_eq!(
            data[2].as_list().expect("text")[1].as_text(),
            Some("две\r\nстроки")
        );
        assert_eq!(parse("{}"), Some(Value::List(Vec::new())));
        for broken in ["{1,2", "{\"open}", "{1,,2}", "{1}}", "{a\"b\"}", ""] {
            assert_eq!(parse(broken), None, "{broken}");
        }
    }

    #[test]
    fn a_record_is_complete_only_when_its_braces_close_outside_strings() {
        let tail = b",\r\n{1,\"a}b{\"\"\",{2}},\r\n{3";
        let first = complete_record(tail).expect("a record").expect("well placed");
        assert_eq!(&tail[first.clone()], b"{1,\"a}b{\"\"\",{2}}");
        assert_eq!(complete_record(&tail[first.end..]), None);
        assert_eq!(complete_record(b"\r\n , "), None);
        assert_eq!(complete_record(b"{\"unterminated}"), None);
        assert_eq!(complete_record(b" x{1}"), Some(Err(1)));
        // Обрыв посреди буквы: запись ещё не полна, и это не ошибка.
        let word = "{\"Ж\"}".as_bytes();
        assert_eq!(complete_record(&word[..3]), None);
    }

    #[test]
    fn the_first_quoted_string_is_found_anywhere() {
        assert_eq!(
            first_quoted("{1,2,\"probe \"\"468\"\", wait\",\"K\"}").as_deref(),
            Some("probe \"468\", wait")
        );
        assert_eq!(first_quoted("garbage"), None);
        assert_eq!(first_quoted("{1,\"open"), None);
    }
}
