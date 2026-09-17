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
            // Завершающая точка — корень DNS, а не часть имени: `localhost.`
            // резолвится в то же самое.
            Host::Name(name) => name.strip_suffix('.').unwrap_or(name) == "localhost",
        }
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
                .unwrap_or_else(|_| Host::Name(name.to_ascii_lowercase())),
        ),
    }
}

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
    fn userinfo_names_the_host_after_it_not_before() {
        assert_eq!(
            host("http://127.0.0.1@evil.com/payload.cfe"),
            Some(Host::Name("evil.com".to_owned()))
        );
        assert!(!loopback("http://127.0.0.1@evil.com/payload.cfe"));
    }
}
