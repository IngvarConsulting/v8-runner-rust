//! Каким адресом клиент открывает базу. Правило одно у `launch` и у клиента тестов:
//! объявленная строка подключения, а без неё — клиентский адрес `infobase.web.url`.
//!
//! Автономная цель здесь не особый случай адреса: её строка подключения — строка прямого
//! шлюза, и клиент идёт по ней, как в кластер, с реквизитами базы. Особая у неё только
//! граница режимов: толстый клиент и обычное приложение против неё не запускаются.

use crate::config::model::{AppConfig, StandaloneWay};
use crate::domain::capability::{Provider, TargetKind};
use crate::domain::launch::LaunchVia;
use crate::platform::enterprise::LaunchClientMode;
use crate::support::error::{AppError, CapabilityReason};

/// Кто идёт к автономной цели без объявленного прямого шлюза — так его называет отказ.
const THIN_CLIENT: &str = "the thin client";

/// Адрес, выбранный для клиента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientAddress<'a> {
    /// `infobase.connection` с реквизитами базы.
    Connection,
    /// `infobase.web.url` как ws-соединение.
    Web(&'a str),
}

impl<'a> ClientAddress<'a> {
    /// Каким адресом открыта база — так его называет ответ.
    pub(crate) const fn via(self) -> LaunchVia {
        match self {
            Self::Connection => LaunchVia::Connection,
            Self::Web(_) => LaunchVia::Web,
        }
    }

    /// Клиентский адрес, если выбран он.
    pub(crate) const fn web_url(self) -> Option<&'a str> {
        match self {
            Self::Connection => None,
            Self::Web(url) => Some(url),
        }
    }
}

/// Толстый клиент и обычное приложение против автономной цели не запускаются; остальные
/// режимы проходят. Отказ — род `capability` с кодом `target`.
pub(crate) fn refuse_a_thick_client_on_a_standalone_target(
    config: &AppConfig,
    mode: LaunchClientMode,
) -> Result<(), AppError> {
    let thick = matches!(mode, LaunchClientMode::Thick | LaunchClientMode::Ordinary);
    if thick && config.target_kind() == TargetKind::Standalone {
        return Err(AppError::capability_for(
            CapabilityReason::Target,
            "the thick client and the ordinary application are not launched against a standalone server: it is opened by the thin client, by the Designer or in a browser",
        ));
    }
    Ok(())
}

/// Адрес клиента: то, что попросили ключом `--via`, иначе строка подключения, а без неё —
/// клиентский адрес. Выбирает только тонкий клиент; у остальных режимов адрес один —
/// строка подключения, — и ключ отвергается.
pub(crate) fn resolve(
    config: &AppConfig,
    mode: LaunchClientMode,
    requested: Option<LaunchVia>,
) -> Result<ClientAddress<'_>, AppError> {
    let thin = matches!(mode, LaunchClientMode::Thin);
    if requested.is_some() && !thin {
        return Err(AppError::Validation(
            "--via selects the address for the thin client; the other launch modes have only one address".to_owned(),
        ));
    }
    let connection_declared = !config.infobase.connection.trim().is_empty();
    match requested {
        Some(LaunchVia::Web) => web_address(config).map(ClientAddress::Web),
        Some(LaunchVia::Connection) if connection_declared => Ok(ClientAddress::Connection),
        Some(LaunchVia::Connection) => Err(direct_gate_undeclared(THIN_CLIENT)),
        None if connection_declared => Ok(ClientAddress::Connection),
        None if thin => match web_address(config) {
            Ok(url) => Ok(ClientAddress::Web(url)),
            Err(_) => Err(AppError::Validation(format!(
                "{}; or declare infobase.web.url, the client address",
                StandaloneWay::DirectGate.undeclared(THIN_CLIENT)
            ))),
        },
        None => Err(direct_gate_undeclared(who(mode))),
    }
}

/// Кто идёт по строке подключения — так его называет отказ.
fn who(mode: LaunchClientMode) -> &'static str {
    match mode {
        LaunchClientMode::Designer => Provider::Designer.as_str(),
        LaunchClientMode::Thin => THIN_CLIENT,
        LaunchClientMode::Thick => "the thick client",
        LaunchClientMode::Ordinary => "the ordinary application",
    }
}

/// Пустая строка подключения бывает только у автономной цели: секция без прямого шлюза.
fn direct_gate_undeclared(who: impl std::fmt::Display) -> AppError {
    AppError::Validation(StandaloneWay::DirectGate.undeclared(who))
}

/// Клиентский адрес цели. Один текст отказа на оба пути: `launch web` и тонкий клиент по
/// вебу отказывают одинаково, потому что не хватает им одного и того же.
pub(crate) fn web_address(config: &AppConfig) -> Result<&str, AppError> {
    config
        .infobase
        .web
        .as_ref()
        .and_then(|web| web.url.as_deref())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            AppError::Validation(
                "infobase.web.url is not declared: the client address appears after `publish` on a web server or is set by hand in infobase.web.url"
                    .to_owned(),
            )
        })
}
