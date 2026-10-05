//! Прежние имена командной строки, принятые на один цикл выпуска.
//!
//! Перечень — единственное место, где названо «прежнее → новое». Разбор держит прежние имена
//! скрытыми в `clap` (`alias`, `hide`), и проверка в этом модуле сверяет перечень с деревом
//! разбора в обе стороны: скрытое имя без строки здесь и строка без скрытого имени — отказ.
//! Страж справки (`tests/cli_help.rs`) читает этот же файл, поэтому новый синоним не
//! появляется ни в справке, ни мимо перечня.
//!
//! Ответ называет лист словаря и тогда, когда вызван прежний путь: `global_flags` берёт
//! новое имя отсюда же. Файл не зависит от остального крейта: интеграционные тесты
//! подключают его по пути.

/// Что именно названо прежним именем.
///
/// Во время работы читаются только имена команд: ответ называет лист словаря. Ключи и
/// значения читают сверка с разбором и страж справки, поэтому вне тестов их поля молчат.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum Previous {
    /// Подкоманда, скрытая или скрытый псевдоним.
    Command(&'static str),
    /// Длинный ключ без `--`, скрытый или скрытый псевдоним.
    Key(&'static str),
    /// Скрытое значение видимого ключа.
    Value {
        /// Длинное имя ключа без `--`.
        key: &'static str,
        /// Прежнее значение.
        value: &'static str,
    },
}

/// Прежнее имя и то, что словарь говорит вместо него.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synonym {
    /// Путь команды словаря, у которой живёт прежнее имя; пустой путь — корень.
    pub command: &'static [&'static str],
    /// Прежнее имя.
    pub previous: Previous,
    /// Запись словаря вместо прежнего имени.
    pub current: &'static str,
}

const fn command(
    command: &'static [&'static str],
    previous: &'static str,
    current: &'static str,
) -> Synonym {
    Synonym {
        command,
        previous: Previous::Command(previous),
        current,
    }
}

const fn key(
    command: &'static [&'static str],
    previous: &'static str,
    current: &'static str,
) -> Synonym {
    Synonym {
        command,
        previous: Previous::Key(previous),
        current,
    }
}

const fn value(
    command: &'static [&'static str],
    key: &'static str,
    previous: &'static str,
    current: &'static str,
) -> Synonym {
    Synonym {
        command,
        previous: Previous::Value {
            key,
            value: previous,
        },
        current,
    }
}

/// Все прежние имена командной строки.
pub const SYNONYMS: &[Synonym] = &[
    command(&[], "bootstrap", "clone"),
    command(&[], "config", "init"),
    command(&[], "build", "push"),
    command(&[], "load", "upload"),
    command(&[], "dump", "pull"),
    command(&[], "syntax", "check"),
    command(&["infobase"], "configuration", "download"),
    command(&["check"], "designer-config", "check"),
    command(&["check"], "designer-modules", "check"),
    command(&["check"], "edt", "check"),
    key(&["clone"], "connection", "--from"),
    key(&["init"], "connection", "--infobase"),
    key(&["push"], "full-rebuild", "--full"),
    key(&["push"], "source-set", "<SET>"),
    key(&["pull"], "source-set", "<SET>"),
    key(&["make"], "source-set", "<SET>"),
    key(&["convert"], "source-set", "<SET>"),
    key(&["pull"], "discard-uncommitted", "--force"),
    key(&["convert"], "discard-uncommitted", "--force"),
    // `incremental` и `partial` значат то же, что без ключа; `full` отказывает и называет
    // `pull --force`.
    key(&["pull"], "mode", "without the key"),
    key(&["test"], "no-build", "--no-push"),
    key(&["upload"], "path", "<FILE>"),
    value(&["upload"], "mode", "merge", "combine"),
    value(&["download"], "state", "working", "without the key"),
    value(&["download"], "state", "database", "db"),
];
