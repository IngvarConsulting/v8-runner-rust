//! Предупреждение ответа: строка для людей и, у части предупреждений, машинный код.
//!
//! `warnings` конверта — строки, и так остаётся. Предупреждение, которое вызывающий должен
//! узнать без разбора текста, едет ещё и в `warning_details` с тем же текстом
//! (`CTR.WIRE.COMMAND-ENVELOPE`). Набор кодов закрыт: новый код приходит с версией конверта.

use serde::Serialize;

use crate::domain::status::HolderStatus;

/// Предупреждение с машинным кодом. Поле `message` равно строке, которая стоит в `warnings`
/// того же ответа; остальные поля — свои у каждого кода.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CodedWarning {
    /// Команда записи идёт на файловой базе, которую держит другая живая рабочая копия
    /// (`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`).
    InfobaseOfAnotherCopy {
        message: String,
        /// Имя базы в местном слое; `null` — база пришла строкой соединения. Поле есть всегда.
        #[schemars(required, extend("type" = ["string", "null"]))]
        name: Option<String>,
        /// Адрес базы без учётных данных, как `address` у `status`: `file:<каталог>`.
        address: String,
        /// Живые копии-владельцы, кроме этой, — та же запись, что `holders.owners` у
        /// `status`.
        owners: Vec<HolderStatus>,
    },
}

impl CodedWarning {
    pub fn message(&self) -> &str {
        match self {
            Self::InfobaseOfAnotherCopy { message, .. } => message,
        }
    }

    /// Код на проводе. Читает сторож закрытого набора.
    #[cfg(test)]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InfobaseOfAnotherCopy { .. } => "infobase_of_another_copy",
        }
    }
}

/// Предупреждение, как его несёт ответ: просто текст или текст с кодом.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    Plain(String),
    Coded(CodedWarning),
}

impl Warning {
    pub fn message(&self) -> &str {
        match self {
            Self::Plain(message) => message,
            Self::Coded(coded) => coded.message(),
        }
    }

    pub fn coded(&self) -> Option<&CodedWarning> {
        match self {
            Self::Plain(_) => None,
            Self::Coded(coded) => Some(coded),
        }
    }
}
