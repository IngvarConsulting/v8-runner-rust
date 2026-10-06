//! Ответ платформы о поколении конфигурации.
//!
//! Конфигуратор пишет ответ `/GetConfigGenerationID` в файл `/Out`, `ibcmd config
//! generation-id` — в stdout, перед ним бывает приглашение ввести пароль, а предупреждения
//! СУБД идут в stderr и ответом не являются
//! ([замер](../../references/1c/confirmed-runtime-measurements.md)). Значение у обоих —
//! последняя непустая строка из сорока шестнадцатеричных знаков. Что иначе — ответа нет:
//! ни совпадения, ни расхождения он не даёт.

use crate::domain::capability::{Provider, TargetKind};

/// Отвечает ли инструмент поколением у такой цели. Единственное место этого признака.
///
/// Конфигуратор у не-файловой цели (кластер) заведомо не отвечает: формат его ответа там не
/// замерен, и нераспознанный ответ — отсутствие ответа
/// (`INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE`, gap #184). Тогда
/// выгрузка поверх каталога памяти о базе не запишет, и отказ без памяти советует полную
/// (`INV.USE-CASES.WITHOUT-A-GENERATION-ANSWER-A-NO-MEMORY-REFUSAL-OFFERS-PULL-FORCE`). Признак
/// снимается вместе с gap #184. Остальные инструменты отвечают.
pub fn answers_generation(tool: Provider, target: TargetKind) -> bool {
    match tool {
        Provider::Designer => target == TargetKind::File,
        Provider::Ibcmd | Provider::Agent | Provider::IbcmdRs | Provider::Webinst => true,
    }
}

/// Число знаков токена поколения.
const TOKEN_LENGTH: usize = 40;

/// Токен поколения из текста ответа: последняя непустая строка, если она — сорок
/// шестнадцатеричных знаков. Метка порядка байтов и пробелы по краям не считаются.
pub fn generation_token(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(|line| line.trim_start_matches('\u{feff}').trim())
        .rfind(|line| !line.is_empty())?;
    (line.len() == TOKEN_LENGTH && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::generation_token;

    #[test]
    fn the_token_is_the_last_non_empty_line_of_forty_hex_digits() {
        let token = "29579ceccaf9ef9b04b0e500bc90ab1aec7370ab";
        assert_eq!(
            generation_token(&format!(
                "Введите пароль для подключения к базе данных: \n{token}\n\n"
            ))
            .as_deref(),
            Some(token)
        );
        assert_eq!(
            generation_token(&format!("\u{feff}{token}\r\n")).as_deref(),
            Some(token),
            "the /Out file of Designer starts with a byte order mark and ends lines with CRLF"
        );
        assert_eq!(
            generation_token("0000000000000000000000000000000000000000").as_deref(),
            Some("0000000000000000000000000000000000000000"),
            "the token of an empty base is a token like any other"
        );
    }

    /// Ответ, которого раннер не узнаёт, — отсутствие ответа: и у файловой базы, и у базы
    /// в кластере, где формат ответа Конфигуратора не замерен.
    #[test]
    fn an_unrecognized_answer_is_no_answer() {
        for text in [
            "",
            "\n\n",
            "Операция не может быть выполнена с текущим составом лицензий.",
            "29579ceccaf9ef9b04b0e500bc90ab1aec7370ab\nОшибка",
            "29579ceccaf9ef9b04b0e500bc90ab1aec7370a",
            "29579ceccaf9ef9b04b0e500bc90ab1aec7370ag",
        ] {
            assert_eq!(generation_token(text), None, "{text:?}");
        }
    }
}
