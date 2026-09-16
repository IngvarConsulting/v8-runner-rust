//! Один владелец маскирования секретов в составленных аргументах.
//!
//! Секрет попадает в argv тремя способами: значением ключа командной строки, который
//! секретен целиком (`/P`, `/WSP`, `--database-password`), сегментом строки соединения
//! внутри одного аргумента (`Srvr=host;Ref=base;Pwd=secret`) и литералом, приклеенным
//! к ключу, которого раннер не знает. Правило живёт здесь, а не у каждой поверхности:
//! показ команды составляют несколько мест, и пропуск в одном означает утечку там,
//! куда не смотрели.

use std::path::Path;

/// Значение, которое встаёт на место секрета.
const MASKED_VALUE: &str = "***";

/// Ключи командной строки, значение которых секретно целиком. Записаны без ведущих
/// `/` и `-`; сравниваются без учёта регистра.
const SECRET_FLAGS: &[&str] = &[
    "p",
    "pwd",
    "ppwd",
    "wsp",
    "uc",
    "accesstoken",
    "configurationrepositoryp",
    "password",
    "database-password",
    "db-pwd",
    "target-database-password",
    "target-db-pwd",
];

/// Ключи командной строки, значение которых называет пользователя. Это не секрет:
/// превью показывает имя, чтобы план читался. Показ отказа его прячет.
const IDENTITY_FLAGS: &[&str] = &[
    "n",
    "wsn",
    "puser",
    "configurationrepositoryn",
    "user",
    "database-user",
    "db-user",
    "target-database-user",
    "target-db-user",
];

/// Секретные параметры строки соединения: пароль базы и пароли веб-сервера и прокси
/// (`IBConnectionString`, «Связи»). Набор отдельный от ключей командной строки —
/// иначе `p=` или `user=` внутри чужого значения маскировались бы зря.
const SECRET_SEGMENTS: &[&str] = &["pwd", "wsp", "wsppwd"];

/// Параметры строки соединения, называющие пользователя.
const IDENTITY_SEGMENTS: &[&str] = &["usr", "wsn", "wspuser"];

/// Что именно скрывает показ аргументов.
#[derive(Clone, Copy)]
enum Hidden {
    /// Только секреты; имя пользователя остаётся читаемым.
    Secrets,
    /// Секреты и имена пользователей.
    SecretsAndIdentities,
}

/// Маскирует секреты в аргументах превью, оставляя имя пользователя читаемым.
///
/// `secrets` несёт значения, о которых известно, что они конфиденциальны: они
/// маскируются везде, где встретятся, даже приклеенными к незнакомому ключу.
pub fn mask_preview_args(args: &[String], secrets: &[&str]) -> Vec<String> {
    mask(args, Hidden::Secrets, secrets)
}

/// Составляет показ команды для отказа и журнала.
///
/// В отличие от превью прячет и имя пользователя: превью человек запросил сам и
/// смотрит сейчас, а текст отказа уходит в журнал CI и живёт дольше самого запуска.
/// Известных литералов у этого показа нет — платформенный слой не видит конфигурации,
/// поэтому секрет, приклеенный к незнакомому ключу, здесь остаётся читаемым.
pub fn render_masked_command(program: &Path, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(program.display().to_string());
    parts.extend(mask(args, Hidden::SecretsAndIdentities, &[]));
    parts.join(" ")
}

fn mask(args: &[String], hidden: Hidden, secrets: &[&str]) -> Vec<String> {
    let mut masked = Vec::with_capacity(args.len());
    let mut mask_detached_value = false;
    let mut mask_address_value = false;
    for arg in args {
        if mask_detached_value {
            mask_detached_value = false;
            masked.push(MASKED_VALUE.to_owned());
            continue;
        }
        if mask_address_value {
            mask_address_value = false;
            masked.push(mask_literals(mask_url_userinfo(arg), secrets));
            continue;
        }
        if is_client_address_key(arg) {
            mask_address_value = true;
            masked.push(arg.clone());
            continue;
        }
        let mut rewritten = String::with_capacity(arg.len());
        // Ключи считаются по словам: в argv платформы значение отделено пробелом, а
        // через `--raw-key` в один аргумент кладут и целую связку вроде
        // `/Proxy -PUser bob -PPwd sec`. Разбор по словам разбирает обе формы одним
        // правилом; `mask_detached_value` переносится и на следующее слово, и на
        // следующий аргумент, потому что значение бывает и там, и там.
        for (word, spacing) in words(arg) {
            if mask_detached_value {
                mask_detached_value = false;
                rewritten.push_str(MASKED_VALUE);
            } else {
                match flag_value_start(word, hidden) {
                    Some(start) if start == word.len() => {
                        mask_detached_value = true;
                        rewritten.push_str(word);
                    }
                    // Значение ключа — весь остаток слова: платформа читает `/P` до
                    // пробела и не делит его по `;`, поэтому `/P=pa;ss` — один пароль.
                    Some(start) => {
                        rewritten.push_str(&word[..start]);
                        rewritten.push_str(MASKED_VALUE);
                    }
                    None => rewritten.push_str(&mask_userinfo(
                        &mask_hidden_key_runs(&mask_segments(word, hidden), hidden),
                        true,
                    )),
                }
            }
            rewritten.push_str(spacing);
        }
        masked.push(mask_literals(rewritten, secrets));
    }
    masked
}

/// Слова аргумента вместе с пробелами, которые шли за каждым: показ должен вернуть
/// строку такой же, какой она была, кроме самих секретов.
fn words(arg: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while cursor < arg.len() {
        let rest = &arg[cursor..];
        let word_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let spacing_len = rest[word_len..]
            .find(|ch: char| !ch.is_whitespace())
            .unwrap_or(rest.len() - word_len);
        found.push((&rest[..word_len], &rest[word_len..word_len + spacing_len]));
        cursor += word_len + spacing_len;
    }
    if found.is_empty() {
        found.push((arg, ""));
    }
    found
}

/// Смещение, с которого начинается значение скрываемого ключа командной строки, или
/// `arg.len()`, когда значение приедет отдельным токеном.
fn flag_value_start(arg: &str, hidden: Hidden) -> Option<usize> {
    let head_len = arg.len() - arg.trim_start_matches(['/', '-']).len();
    if head_len == 0 {
        return None;
    }
    let rest = &arg[head_len..];
    let key_len = matching_flag_len(rest, hidden)?;
    let tail = &rest[key_len..];
    if tail.is_empty() {
        return Some(arg.len());
    }
    // Приклеенный `/Psecret` неотличим от постороннего ключа вроде `/Proxy`, поэтому
    // значением считается только то, что отделено явным разделителем; приклеенную
    // форму закрывает правило литералов, когда значение секрета известно.
    let separator = tail.chars().next()?;
    if matches!(separator, ' ' | '=' | ':' | '"') {
        return Some(head_len + key_len + separator.len_utf8());
    }
    None
}

/// Длина ключа, которым начинается `rest`, если это ключ скрываемого вида.
fn matching_flag_len(rest: &str, hidden: Hidden) -> Option<usize> {
    flags(hidden)
        .filter(|key| {
            // `get`, а не срез: ключ считается в байтах, а аргумент бывает
            // многобайтным — `/Пароль` не должен ронять показ команды. Регистр
            // гасится по ASCII, чтобы длина ключа не разъехалась с длиной значения.
            rest.get(..key.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(key))
        })
        // Длиннейший ключ побеждает: иначе `/password=x` разобрался бы как ключ `p`
        // с разделителем `a`, не нашёл бы разделителя и напечатал пароль.
        .map(str::len)
        .max()
}

fn flags(hidden: Hidden) -> impl Iterator<Item = &'static str> {
    let identities = match hidden {
        Hidden::Secrets => [].as_slice(),
        Hidden::SecretsAndIdentities => IDENTITY_FLAGS,
    };
    SECRET_FLAGS.iter().chain(identities.iter()).copied()
}

fn segment_keys(hidden: Hidden) -> impl Iterator<Item = &'static str> {
    let identities = match hidden {
        Hidden::Secrets => [].as_slice(),
        Hidden::SecretsAndIdentities => IDENTITY_SEGMENTS,
    };
    SECRET_SEGMENTS.iter().chain(identities.iter()).copied()
}

fn is_hidden_segment_key(key: &str, hidden: Hidden) -> bool {
    segment_keys(hidden).any(|known| key.eq_ignore_ascii_case(known))
}

/// Маскирует значения скрываемых параметров строки соединения внутри одного аргумента.
fn mask_segments(arg: &str, hidden: Hidden) -> String {
    let (found, quotes_are_balanced) = segments(arg);
    let mut masked = String::with_capacity(arg.len());
    for (index, segment) in found.into_iter().enumerate() {
        if index > 0 {
            masked.push(';');
        }
        let Some(separator) = segment.find('=') else {
            masked.push_str(segment);
            continue;
        };
        if is_hidden_segment_key(segment[..separator].trim(), hidden) {
            masked.push_str(&segment[..=separator]);
            masked.push_str(MASKED_VALUE);
            if !quotes_are_balanced {
                // Кавычка не закрыта, значит где кончается значение — неизвестно.
                // Дальше молчание: `Pwd="sec;ret` кончается не на `;`, и `ret` —
                // всё ещё половина пароля.
                return masked;
            }
        } else {
            masked.push_str(segment);
        }
    }
    masked
}

/// Разбивает аргумент по `;`, не считая разделителем точку с запятой внутри кавычек:
/// платформа разрешает `Pwd="sec;ret"`, и половина такого пароля — всё ещё утечка.
///
/// Незакрытая кавычка ломает правило в опасную сторону — весь хвост становится одним
/// значением, — поэтому такой аргумент разбирается так, будто кавычек в нём нет, и
/// вызывающий узнаёт об этом вторым значением.
fn segments(value: &str) -> (Vec<&str>, bool) {
    let quoted = split(value, true);
    if quoted
        .iter()
        .any(|segment| segment.matches('"').count() % 2 == 1)
    {
        return (split(value, false), false);
    }
    (quoted, true)
}

fn split(value: &str, honour_quotes: bool) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' if honour_quotes => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                found.push(&value[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    found.push(&value[start..]);
    found
}

/// Прячет пароль в объявленном клиентском адресе, оставляя сам адрес узнаваемым.
///
/// Адрес приходит из `infobase.web.url`, которое не валидируется, поэтому схемы в нём
/// может не быть: раз это заведомо адрес, схема и не требуется.
pub fn mask_url_userinfo(value: &str) -> String {
    mask_userinfo(value, false)
}

/// Ключ, за которым идёт клиентский адрес: его значение маскируется как адрес,
/// а не целиком — адрес человеку нужен, чтобы понять, куда раннер собрался.
fn is_client_address_key(arg: &str) -> bool {
    arg.strip_prefix('/')
        .or_else(|| arg.strip_prefix('-'))
        .is_some_and(|rest| rest.eq_ignore_ascii_case("ws"))
}

/// Прячет пароль из userinfo адреса: `http://alice:pass@host/base` → `http://alice:***@host/base`.
///
/// Маскируется только то, что после `:`. Голое имя пользователя секретом не является, а
/// спрятать его целиком значит сделать адрес неузнаваемым.
///
/// `require_scheme` разделяет два случая. У значения `/WS` схемы может не быть вовсе —
/// поле `infobase.web.url` не валидируется, — поэтому там адресом считается и голый
/// authority. В любом другом аргументе без схемы на адрес похож и путь вида
/// `C:\dir@host`, и маскировать его значило бы портить читаемое.
fn mask_userinfo(value: &str, require_scheme: bool) -> String {
    let authority_start = match value.find("://") {
        Some(scheme_end) => scheme_end + "://".len(),
        None if require_scheme => return value.to_owned(),
        None if value.starts_with("//") => "//".len(),
        None => 0,
    };
    let authority_end = value[authority_start..]
        .find(['/', '?', '#'])
        .map_or(value.len(), |at| authority_start + at);
    let authority = &value[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return value.to_owned();
    };
    let Some(colon) = authority[..at].find(':') else {
        return value.to_owned();
    };
    format!(
        "{}{}:{MASKED_VALUE}{}",
        &value[..authority_start],
        &authority[..colon],
        &value[authority_start + at..]
    )
}

/// Второй проход, закрывающий то, что разбор по сегментам не увидел.
///
/// Кавычки в строке соединения стоят не только вокруг значения: платформа велит брать
/// в кавычки строку целиком, удваивая внутренние. Тогда весь аргумент — один сегмент с
/// ключом `"Srvr`, и пароль в нём уцелел бы. Поэтому после разбора ключ ищется ещё раз
/// — по границе слева и `=` справа, — и его значение маскируется до ближайшей `;`.
/// Проход идемпотентен: уже замаскированное значение он маскирует в себя же.
fn mask_hidden_key_runs(word: &str, hidden: Hidden) -> String {
    // Регистр гасится по ASCII: длина обязана совпадать с длиной оригинала, иначе
    // найденные здесь смещения не годятся для нарезки `word`.
    let lowered = word.to_ascii_lowercase();
    let mut masked = String::with_capacity(word.len());
    let mut cursor = 0;
    while let Some(value_start) = next_hidden_key_value(&lowered, cursor, hidden) {
        let value_end = word[value_start..]
            .find(';')
            .map_or(word.len(), |at| value_start + at);
        masked.push_str(&word[cursor..value_start]);
        masked.push_str(MASKED_VALUE);
        cursor = value_end;
    }
    masked.push_str(&word[cursor..]);
    masked
}

/// Смещение значения ближайшего скрываемого ключа строки соединения после `from`.
fn next_hidden_key_value(lowered: &str, from: usize, hidden: Hidden) -> Option<usize> {
    segment_keys(hidden)
        .filter_map(|key| {
            let mut search = from;
            while let Some(found) = lowered[search..].find(key) {
                let key_start = search + found;
                let value_start = key_start + key.len();
                let at_boundary = key_start == 0
                    || matches!(
                        lowered[..key_start].chars().next_back(),
                        Some(';' | ' ' | '\'' | '"')
                    );
                if at_boundary && lowered[value_start..].starts_with('=') {
                    return Some(value_start + 1);
                }
                search = value_start;
            }
            None
        })
        .min()
}

/// Маскирует известные конфиденциальные литералы, где бы они ни встретились.
fn mask_literals(arg: String, secrets: &[&str]) -> String {
    let mut masked = arg;
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        if masked.contains(secret) {
            masked = masked.replace(secret, MASKED_VALUE);
        }
    }
    masked
}

#[cfg(test)]
mod tests {
    use super::{mask_preview_args, mask_url_userinfo, render_masked_command};
    use std::path::Path;

    fn preview(args: &[&str], secrets: &[&str]) -> Vec<String> {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        mask_preview_args(&owned, secrets)
    }

    fn rendered(args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        render_masked_command(Path::new("1cv8c"), &owned)
    }

    #[test]
    fn masks_the_detached_password_value_and_keeps_the_user_readable() {
        let args = preview(
            &["ENTERPRISE", "/N", "Администратор", "/P", "s3cret"],
            &["s3cret"],
        );
        assert_eq!(args, vec!["ENTERPRISE", "/N", "Администратор", "/P", "***"]);
    }

    #[test]
    fn masks_every_attached_password_separator_form() {
        for arg in ["/P=s3cret", "/P:s3cret", "-p=s3cret", "/P s3cret"] {
            let args = preview(&[arg], &[]);
            assert!(
                !args[0].contains("s3cret") && args[0].ends_with("***"),
                "{arg} -> {}",
                args[0]
            );
        }
    }

    #[test]
    fn keeps_unrelated_keys_that_merely_start_with_a_hidden_key() {
        let args = preview(&["/Proxy", "/PublishWSOnDemand", "/Personal=area"], &[]);
        assert_eq!(args, vec!["/Proxy", "/PublishWSOnDemand", "/Personal=area"]);
    }

    #[test]
    fn masks_only_the_password_segment_of_a_connection_string() {
        let args = preview(
            &[
                "/IBConnectionString",
                "Srvr=\"srv:1541\";Ref=\"ut\";Usr=Админ;Pwd=s3cret;",
            ],
            &[],
        );
        assert_eq!(args[1], "Srvr=\"srv:1541\";Ref=\"ut\";Usr=Админ;Pwd=***;");
    }

    #[test]
    fn masks_a_password_quoted_around_a_semicolon() {
        let args = preview(&["File=/tmp/ib;Pwd=\"sec;ret\";Locale=ru"], &[]);
        assert_eq!(args[0], "File=/tmp/ib;Pwd=***;Locale=ru");
    }

    /// Незакрытая кавычка не должна прятать `;` от разбора: иначе весь хвост строки
    /// соединения станет одним значением и пароль в нём уцелеет.
    #[test]
    fn masks_the_password_after_an_unbalanced_quote() {
        let args = preview(&["Srvr=host;Ref=\"ut;Pwd=s3cret"], &[]);
        assert!(!args[0].contains("s3cret"), "{}", args[0]);
        // Где кончается значение — неизвестно, поэтому хвост не печатается вовсе.
        assert_eq!(
            preview(&["Srvr=h;Pwd=\"sec;ret"], &[]),
            vec!["Srvr=h;Pwd=***"]
        );
    }

    /// Значение `/P` — весь остаток слова: платформа читает его до пробела и не делит
    /// по `;`, поэтому пароль с точкой с запятой маскируется целиком, а не наполовину.
    #[test]
    fn masks_a_password_that_itself_contains_a_semicolon() {
        assert_eq!(preview(&["/P=pa;ss"], &[]), vec!["/P=***"]);
        assert_eq!(preview(&["/P=s3cret;Pwd=other"], &[]), vec!["/P=***"]);
    }

    /// Платформа велит брать строку соединения в кавычки целиком, удваивая внутренние
    /// (`IBConnectionString`, «Связи»). Тогда весь аргумент — один сегмент с ключом
    /// `"Srvr`, и разбор по сегментам пароля в нём не видит.
    #[test]
    fn masks_the_password_of_a_connection_string_quoted_as_a_whole() {
        for arg in [
            "\"Srvr=host;Ref=base;Usr=alice;Pwd=s3cret\"",
            "\"Srvr=\"\"srv:1541\"\";Ref=\"\"ut\"\";Pwd=s3cret\"",
            "Srvr=host;Ref=\"ut;Usr=alice;Pwd=s3cret\";Locale=ru",
        ] {
            let args = preview(&[arg], &[]);
            assert!(!args[0].contains("s3cret"), "{arg} -> {}", args[0]);
        }
    }

    /// В один аргумент через `--raw-key` кладут и целую связку ключей: значение
    /// ищется по каждому слову, а не только по первому.
    #[test]
    fn masks_a_key_that_is_not_the_first_word_of_an_argument() {
        assert_eq!(
            preview(&["/Proxy -PSrv 10.0.0.1 -PPort 3128 -PPwd s3cret"], &[]),
            vec!["/Proxy -PSrv 10.0.0.1 -PPort 3128 -PPwd ***"]
        );
        assert_eq!(
            preview(&["/N alice /P s3cret"], &[]),
            vec!["/N alice /P ***"]
        );
    }

    /// Значение скрываемого ключа приезжает следующим аргументом и тогда, когда ключ
    /// закрыл собой предыдущий.
    #[test]
    fn masks_a_detached_value_that_arrives_in_the_next_argument() {
        assert_eq!(
            preview(&["/AccessToken", "jwt", "/Out", "/tmp/log"], &[]),
            vec!["/AccessToken", "***", "/Out", "/tmp/log"]
        );
        assert_eq!(preview(&["/P"], &[]), vec!["/P"]);
    }

    #[test]
    fn masks_the_web_server_password_of_a_connection_string() {
        let args = preview(&["Srvr=host;wsn=alice;wsp=s3cret;wsppwd=proxy-s3cret"], &[]);
        assert_eq!(args[0], "Srvr=host;wsn=alice;wsp=***;wsppwd=***");
    }

    #[test]
    fn masks_a_known_secret_glued_to_an_unrecognised_key() {
        let args = preview(&["/Pses3cret"], &["s3cret"]);
        assert_eq!(args, vec!["/Pse***"]);
    }

    #[test]
    fn masks_the_web_server_password_key() {
        assert_eq!(preview(&["/WSP", "s3cret"], &[]), vec!["/WSP", "***"]);
        assert_eq!(preview(&["/WSP=s3cret"], &[]), vec!["/WSP=***"]);
        assert_eq!(
            preview(&["/AccessToken", "jwt"], &[]),
            vec!["/AccessToken", "***"]
        );
    }

    #[test]
    fn a_rendered_command_hides_the_user_as_well_as_the_password() {
        assert_eq!(
            rendered(&["/N", "alice", "/P", "s3cret"]),
            "1cv8c /N *** /P ***"
        );
    }

    #[test]
    fn a_rendered_command_keeps_the_readable_part_of_a_connection_string() {
        let line = rendered(&[
            "/IBConnectionString",
            "Srvr=host;Ref=base;Usr=alice;Pwd=s3cret",
        ]);
        assert_eq!(
            line,
            "1cv8c /IBConnectionString Srvr=host;Ref=base;Usr=***;Pwd=***"
        );
    }

    /// Ключи строки соединения — отдельный набор: `n=` и `user=` внутри чужого
    /// значения не пароль и не имя пользователя базы, и маскировать их незачем.
    #[test]
    fn keeps_segment_keys_that_are_only_command_line_keys() {
        assert_eq!(
            rendered(&["Filter=n=5;user=bob"]),
            "1cv8c Filter=n=5;user=bob"
        );
    }

    /// Пароль пользователя хранилища приезжает тем же путём, что `/WSP`.
    #[test]
    fn masks_the_repository_password_and_access_code() {
        assert_eq!(
            preview(
                &[
                    "/ConfigurationRepositoryN",
                    "bob",
                    "/ConfigurationRepositoryP",
                    "s3cret"
                ],
                &[]
            ),
            vec![
                "/ConfigurationRepositoryN",
                "bob",
                "/ConfigurationRepositoryP",
                "***"
            ]
        );
        assert_eq!(preview(&["/UC=s3cret"], &[]), vec!["/UC=***"]);
    }

    /// Разбор адреса написан руками, поэтому таблица форм: со схемой и без неё, IPv6,
    /// `@` в пути и запросе, пустой пароль, голое имя пользователя, повторное применение.
    #[test]
    fn mask_url_userinfo_hides_the_password_in_every_shape_of_address() {
        for (value, expected) in [
            (
                "http://alice:s3cret@host/base",
                "http://alice:***@host/base",
            ),
            (
                "https://alice:s3cret@host:443/b?x=1#f",
                "https://alice:***@host:443/b?x=1#f",
            ),
            // Схемы может не быть вовсе: поле не валидируется.
            ("//alice:s3cret@host/base", "//alice:***@host/base"),
            ("alice:s3cret@host/base", "alice:***@host/base"),
            // Пароль с разделителями внутри: маскируется от первого `:` до последней `@`.
            ("http://alice:p@ss:word@host/b", "http://alice:***@host/b"),
            (
                "http://alice:s3cret@[2001:db8::1]:8080/b",
                "http://alice:***@[2001:db8::1]:8080/b",
            ),
            // Прятать нечего.
            ("http://alice@host/base", "http://alice@host/base"),
            ("http://host/base", "http://host/base"),
            (
                "http://[2001:db8::1]:8080/base",
                "http://[2001:db8::1]:8080/base",
            ),
            ("http://host/path@with-at", "http://host/path@with-at"),
            ("http://host/base?q=a@b", "http://host/base?q=a@b"),
            // Уже замаскированное второй раз не портится.
            ("http://alice:***@host/base", "http://alice:***@host/base"),
        ] {
            assert_eq!(mask_url_userinfo(value), expected, "вход: {value}");
        }
    }

    /// Маскируется значение `/WS`, а не всё, что похоже на адрес: путь с `@` и строка
    /// подключения остаются читаемыми.
    #[test]
    fn only_the_client_address_is_masked_as_an_address() {
        let masked = preview(
            &[
                "/WS",
                "http://alice:s3cret@host/base",
                "/C",
                "C:\\dir@host",
                "/IBConnectionString",
                "Srvr=host;Ref=base",
            ],
            &[],
        );

        assert_eq!(masked[1], "http://alice:***@host/base");
        assert_eq!(
            masked[3], "C:\\dir@host",
            "путь не адрес и портиться не должен"
        );
        assert_eq!(masked[5], "Srvr=host;Ref=base");
    }

    #[test]
    fn a_multibyte_value_does_not_split_a_character() {
        let args = preview(&["/P=пароль", "Srvr=сервер;Pwd=пароль"], &[]);
        assert_eq!(args, vec!["/P=***", "Srvr=сервер;Pwd=***"]);
    }
}
