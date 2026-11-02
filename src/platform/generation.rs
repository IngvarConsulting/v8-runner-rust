//! Ответ платформы о поколении конфигурации.
//!
//! Конфигуратор пишет ответ `/GetConfigGenerationID` в файл `/Out`, `ibcmd config
//! generation-id` — в stdout, перед ним бывает приглашение ввести пароль, а предупреждения
//! СУБД идут в stderr и ответом не являются
//! ([замер](../../references/1c/confirmed-runtime-measurements.md)). Значение у обоих —
//! последняя непустая строка из сорока шестнадцатеричных знаков. Что иначе — ответа нет:
//! ни совпадения, ни расхождения он не даёт.

use std::path::Path;

use crate::domain::capability::{Provider, TargetKind};
use crate::platform::locator::PlatformVersion;

/// Отвечает ли инструмент поколением у такой цели. Единственное место этого признака.
///
/// Конфигуратор у не-файловой цели (кластер, прямой шлюз автономного сервера) заведомо не
/// отвечает: формат его ответа там не замерен, и нераспознанный ответ — отсутствие ответа
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

/// Платформа, первой знающая команду поколения: `/GetConfigGenerationID` пакетного
/// Конфигуратора и `config generation-id` режима агента реализованы в 8.3.20 (список
/// изменений платформы 8.3.20, V8Update.htm). Платформа старше команды ключ не узнаёт:
/// пакетной операции в запуске нет, Конфигуратор открывает окно в интерактивном режиме и
/// сам не кончается ([замер](../../references/1c/confirmed-runtime-measurements.md)
/// 03.11.2026 на 8.3.12).
const GENERATION_ID_FIRST_PLATFORM: PlatformVersion = PlatformVersion {
    major: 8,
    minor: 3,
    patch: 20,
    build: 0,
};

/// Знает ли утилита с таким путём команду поколения
/// (`INV.PLATFORM.A-PLATFORM-WITHOUT-THE-GENERATION-COMMAND-IS-NOT-ASKED`). Версия читается
/// из пути установки — компонент вида `8.3.12.1855`, как его читает и поиск утилит; самая
/// глубокая из годных. Версии в пути нет — вопрос задают: от запуска без конца, каким
/// отвечает платформа без команды, чтение защищено собственным пределом
/// (`INV.USE-CASES.A-GENERATION-READ-DECLARES-ITS-OWN-CAP`), ответа нет.
pub fn knows_generation_id(binary: &Path) -> bool {
    binary
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(PlatformVersion::parse_strict)
        .is_none_or(|version| version >= GENERATION_ID_FIRST_PLATFORM)
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
    use super::{generation_token, knows_generation_id};
    use std::path::{Path, PathBuf};

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

    /// Команда поколения реализована в 8.3.20: платформу старше не спрашивают, новее —
    /// спрашивают, а версии в пути нет — спрашивают, защищённые пределом чтения.
    #[test]
    fn a_platform_older_than_8_3_20_knows_no_generation_command() {
        let designer_of = |version: &str| {
            PathBuf::from("C:")
                .join("Program Files")
                .join("1cv8")
                .join(version)
                .join("bin")
                .join("1cv8.exe")
        };
        assert!(!knows_generation_id(&designer_of("8.3.12.1855")));
        assert!(!knows_generation_id(&designer_of("8.3.19.9999")));
        assert!(knows_generation_id(&designer_of("8.3.20.2639")));
        assert!(knows_generation_id(&designer_of("8.3.27.2074")));
        assert!(knows_generation_id(Path::new(
            "/opt/1cv8/x86_64/8.5.4.1878/1cv8"
        )));

        let ibcmd_of = |version: &str| {
            PathBuf::from("/opt/1cv8/x86_64")
                .join(version)
                .join("ibcmd")
        };
        assert!(!knows_generation_id(&ibcmd_of("8.3.13.1809")));
        assert!(knows_generation_id(&ibcmd_of("8.3.20.2639")));

        assert!(
            knows_generation_id(Path::new("1cv8.exe")),
            "a versionless path is asked anyway: the read cap answers for it"
        );
        let nested = PathBuf::from("8.3.12.1855")
            .join("portable")
            .join("8.3.27.2074")
            .join("1cv8.exe");
        assert!(
            knows_generation_id(&nested),
            "the deepest version-like component wins"
        );
    }
}
