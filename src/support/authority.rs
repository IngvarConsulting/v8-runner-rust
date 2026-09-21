//! Разбор authority-части адреса.
//!
//! Модуль существует, чтобы у вопроса «это петля?» был один ответ на весь раннер.
//! Самодельные проверки по подстроке здесь запрещены: `127.evil.com` начинается с
//! `127.`, а `127.0.0.1@evil.com` — целиком с петлевого адреса, и оба ведут наружу.
//! Разбор отдан парсеру URL, который знает про userinfo, скобки IPv6, порт и IDN.

use std::net::IpAddr;
use std::str::FromStr;

use url::Url;

/// Хост, каким его назвали в адресе.
///
/// `PartialEq` здесь структурное: оно сравнивает записи, а не машины, на которые
/// те указывают. Вопрос «та же машина?» задают предикатам вроде [`Host::is_loopback`],
/// которые приводят адрес к канонической форме.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// Числовой адрес.
    Address(IpAddr),

    /// Имя, приведённое к нижнему регистру.
    Name(String),
}

impl Host {
    /// Петлевой ли хост.
    ///
    /// Сравнение идёт по канонической форме: `::ffff:127.0.0.1` — это тот же
    /// `127.0.0.1`, записанный по-другому, но `Ipv6Addr::is_loopback` знает только
    /// `::1` и на отображённом адресе отвечает «нет».
    pub fn is_loopback(&self) -> bool {
        match self {
            Host::Address(address) => address.to_canonical().is_loopback(),
            Host::Name(name) => name == "localhost",
        }
    }

    /// Имя из строки, приведённое к сравнимому виду.
    ///
    /// Завершающая точка — корень DNS, а не часть имени, и снимается здесь, при
    /// постройке. Если снимать её в предикате, `PartialEq` останется структурным:
    /// `runner.` перестанет совпадать с `runner`, хотя `localhost.` — с `localhost`
    /// совпадёт. Такая асимметрия бьёт ровно по спискам разрешённых имён.
    fn name(value: &str) -> Self {
        let value = value.to_ascii_lowercase();
        let trimmed = value.strip_suffix('.').unwrap_or(&value);
        Host::Name(trimmed.to_owned())
    }
}

/// Хост из разобранного адреса.
pub fn host_of_url(url: &Url) -> Option<Host> {
    match url.host()? {
        url::Host::Ipv4(address) => Some(Host::Address(IpAddr::V4(address))),
        url::Host::Ipv6(address) => Some(Host::Address(IpAddr::V6(address))),
        // Числовой адрес разбирается и здесь: для схем вне списка специальных
        // парсер URL отдаёт `127.0.0.1` как имя, и без этой ветки числовой адрес
        // отвечал бы на вопрос о петле «нет». Ошибиться в эту сторону нельзя:
        // у предиката будут и те, кто по петле пропускает, и те, кто её запрещает.
        url::Host::Domain(name) => Some(
            IpAddr::from_str(name)
                .map(Host::Address)
                .unwrap_or_else(|_| Host::name(name)),
        ),
    }
}

/// Хост из authority — значения заголовка `Host` или записи `host:port`.
///
/// Разбор идёт тем же парсером, что и для полного адреса: authority достраивается
/// до `http://<authority>/` (см. [`url_of_authority`]).
pub fn host_of_authority(authority: &str) -> Option<Host> {
    host_of_url(&url_of_authority(authority)?)
}

/// Хост и порт из записи `host[:port]` — адреса, который уходит утилите платформы как
/// есть (`rac`, `ras cluster`). Читается тем же парсером, что [`host_of_authority`]:
/// IPv6 — только в скобках (`[::1]:1545`), голый `::1` не разбирается; порт 0 — не адрес.
/// Порт 80 парсер прячет как умолчание достроенной схемы, поэтому он читается из записи.
pub fn host_and_port_of_authority(authority: &str) -> Option<(Host, Option<u16>)> {
    // `srv:` парсер URL читает как адрес без порта; для записи, которая уйдёт утилите
    // как есть, пустой порт — опечатка, а не умолчание.
    if authority.ends_with(':') {
        return None;
    }
    let url = url_of_authority(authority)?;
    let host = host_of_url(&url)?;
    let port = url.port().or_else(|| {
        authority
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .filter(|port| *port == 80)
    });
    if port == Some(0) {
        return None;
    }
    Some((host, port))
}

/// Authority, достроенная до адреса. Перед достройкой отсекается всё, чего в заголовке
/// `Host` быть не может: иначе `evil.com@127.0.0.1` достроился бы в адрес с userinfo
/// `evil.com` и петлевым хостом — то есть ровно в ту подмену, от которой разбор и защищает.
fn url_of_authority(authority: &str) -> Option<Url> {
    let unusable = authority.is_empty()
        || authority.len() > MAX_AUTHORITY_LEN
        || authority
            .chars()
            .any(|symbol| FORBIDDEN_IN_AUTHORITY.contains(&symbol) || symbol.is_control());
    if unusable {
        return None;
    }

    Url::parse(&format!("http://{authority}/")).ok()
}

/// Длина с запасом: имя в DNS не длиннее 253 октетов, порт добавляет ещё шесть.
const MAX_AUTHORITY_LEN: usize = 260;

/// Ничего из этого в `Host` быть не может, а при достройке до адреса меняет смысл.
const FORBIDDEN_IN_AUTHORITY: &[char] = &['@', '/', '\\', '?', '#', ' ', '\t'];

#[cfg(test)]
mod tests {
    use super::*;

    fn host(url: &str) -> Option<Host> {
        Url::parse(url).ok().as_ref().and_then(host_of_url)
    }

    fn loopback(url: &str) -> bool {
        host(url).is_some_and(|host| host.is_loopback())
    }

    #[test]
    fn loopback_is_recognised_in_every_spelling_it_has() {
        for url in [
            "http://127.0.0.1/x",
            "http://127.0.0.1:3000/x",
            "http://127.1.2.3:3000/x",
            "http://[::1]/x",
            "http://[::1]:8080/x",
            "http://[::ffff:127.0.0.1]:80/x",
            "http://localhost/x",
            "http://LOCALHOST:3000/x",
            // Восьмеричная и целочисленная записи тех же четырёх байт. Разбор
            // адреса сводит их к `127.0.0.1` ровно так же, как это сделает
            // резолвер, — значит и решение о петле должно быть тем же.
            "http://0177.0.0.1/x",
            "http://2130706433/x",
        ] {
            assert!(loopback(url), "{url} is loopback");
        }
    }

    #[test]
    fn a_name_that_merely_starts_like_the_loopback_address_is_not_it() {
        // Ровно эти записи проходили проверку по подстроке: обе начинаются с
        // `127.`, а ведут на чужой хост. Имя вида `127.0.0.1.nip.io` отдаёт
        // любому желающему публичный сервис подстановочного DNS.
        for url in [
            "http://127.evil.com/payload.cfe",
            "http://127.0.0.1.nip.io/payload.cfe",
            "http://example.com/payload.cfe",
        ] {
            assert!(!loopback(url), "{url} is not loopback");
        }
    }

    #[test]
    fn the_root_dot_is_not_part_of_the_name() {
        assert!(loopback("http://localhost./x"));
        assert!(loopback("http://127.0.0.1./x"));
        assert!(!loopback("http://localhost.evil.com./x"));
        // Снято при постройке, поэтому и равенство записей это видит: список
        // разрешённых имён сравнивают именно им.
        assert_eq!(host("http://runner./x"), host("http://runner/x"));
        assert_eq!(
            host_of_authority("runner.:3000"),
            host_of_authority("runner")
        );
    }

    #[test]
    fn a_numeric_host_stays_numeric_outside_the_special_schemes() {
        // Для схем вне списка специальных парсер URL отдаёт адрес как имя.
        // Ответ про петлю от этого меняться не должен.
        assert_eq!(
            host("redis://127.0.0.1:6379/0"),
            Some(Host::Address("127.0.0.1".parse().expect("literal address")))
        );
        assert!(loopback("redis://127.0.0.1:6379/0"));
        assert!(!loopback("redis://127.evil.com:6379/0"));
    }

    #[test]
    fn an_authority_is_read_the_same_way_a_full_address_is() {
        for authority in [
            "127.0.0.1",
            "127.0.0.1:3000",
            "[::1]:8080",
            "localhost",
            "LOCALHOST:3000",
        ] {
            assert!(
                host_of_authority(authority).is_some_and(|host| host.is_loopback()),
                "{authority} is loopback"
            );
        }
        for authority in ["127.evil.com", "127.0.0.1.nip.io:3000", "example.com"] {
            assert!(
                host_of_authority(authority).is_some_and(|host| !host.is_loopback()),
                "{authority} is not loopback"
            );
        }
    }

    #[test]
    fn an_authority_that_cannot_appear_in_the_header_is_refused() {
        // `evil.com@127.0.0.1` — та самая подмена, ради которой отсев и стоит:
        // при достройке до адреса петлевым хостом стал бы правый край.
        for authority in [
            "",
            "evil.com@127.0.0.1",
            "127.0.0.1/../evil",
            "127.0.0.1 evil.com",
            "127.0.0.1:99999",
            "127.0.0.1\u{0}",
            "[::1",
            "127.0.0.1?x",
        ] {
            assert!(
                host_of_authority(authority).is_none(),
                "{authority:?} is refused"
            );
        }
    }

    /// Запись `host[:port]` читается так же, как заголовок: порт — если назван, IPv6 — в
    /// скобках. Голый `::1` парсер по двоеточиям не режет, порт 0 адресом не считается.
    #[test]
    fn a_host_with_an_optional_port_is_read_as_the_header_is() {
        let name = |name: &str| Host::Name(name.to_owned());
        let address = |address: &str| Host::Address(address.parse().expect("literal address"));
        for (authority, expected) in [
            ("srv", (name("srv"), None)),
            ("srv:1545", (name("srv"), Some(1545))),
            ("SRV.example.:1540", (name("srv.example"), Some(1540))),
            ("10.0.0.5:1545", (address("10.0.0.5"), Some(1545))),
            ("[::1]:1540", (address("::1"), Some(1540))),
            ("[::1]", (address("::1"), None)),
            ("srv:80", (name("srv"), Some(80))),
            ("srv:080", (name("srv"), Some(80))),
        ] {
            assert_eq!(
                host_and_port_of_authority(authority),
                Some(expected),
                "{authority}"
            );
        }
        for authority in [
            "",
            "::1",
            ":1545",
            "srv:",
            "srv:0",
            "srv:x",
            "srv:99999",
            "[::1",
        ] {
            assert!(
                host_and_port_of_authority(authority).is_none(),
                "{authority:?} is refused"
            );
        }
    }

    #[test]
    fn userinfo_names_the_host_after_it_not_before() {
        assert_eq!(
            host("http://127.0.0.1@evil.com/payload.cfe"),
            Some(Host::Name("evil.com".to_owned()))
        );
        assert!(!loopback("http://127.0.0.1@evil.com/payload.cfe"));
    }
}
