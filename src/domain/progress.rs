//! Словарь хода долгой команды: фаза шага, вид строки агента и причина отмены
//! (`CTR.WIRE.PROGRESS-EVENTS`).
//!
//! Сценарий называет фазу шага, транспорт событий её печатает: словарь один на обе стороны,
//! и через журнал он идёт строкой из [`ProgressPhase::as_str`], а обратно разбирается
//! [`ProgressPhase::parse`].

use serde::Serialize;

/// Что делает шаг долгой команды. Перечень закрыт: новая фаза меняет форму событий.
///
/// С шагами итогового конверта (`steps`) фаза не сверяется: шаг хода — то, что раннер
/// начал делать, а шаг конверта — то, чем это кончилось.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    /// Превью: показывает план и ничего не запускает.
    Preview,
    /// Открытие сессии исполнителя — агента Конфигуратора.
    Session,
    /// Ожидание: живой агент ещё поднимается или занят.
    Wait,
    /// Чтение базы без изменений: совместимость, установленные расширения, найденные изменения.
    Inspect,
    /// Вопрос исполнителю, чем будет выгрузка.
    Forecast,
    /// Преобразование формата исходников средствами EDT.
    Convert,
    /// Выгрузка конфигурации в файлы.
    Dump,
    /// Загрузка в базу.
    Load,
    /// Обновление конфигурации базы данных.
    UpdateDb,
    /// Возврат к конфигурации базы данных.
    Reset,
    /// Действие над расширением базы.
    Extensions,
    /// Проверка синтаксиса или проекта.
    Check,
    /// Сборка или выгрузка пакета конфигурации.
    Package,
    /// Создание, снимок или восстановление базы.
    Infobase,
    /// Прогон тестов.
    Test,
    /// Запуск клиента.
    Launch,
    /// Публикация на веб-сервере.
    Publish,
}

impl ProgressPhase {
    pub const ALL: [Self; 17] = [
        Self::Preview,
        Self::Session,
        Self::Wait,
        Self::Inspect,
        Self::Forecast,
        Self::Convert,
        Self::Dump,
        Self::Load,
        Self::UpdateDb,
        Self::Reset,
        Self::Extensions,
        Self::Check,
        Self::Package,
        Self::Infobase,
        Self::Test,
        Self::Launch,
        Self::Publish,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Session => "session",
            Self::Wait => "wait",
            Self::Inspect => "inspect",
            Self::Forecast => "forecast",
            Self::Convert => "convert",
            Self::Dump => "dump",
            Self::Load => "load",
            Self::UpdateDb => "update_db",
            Self::Reset => "reset",
            Self::Extensions => "extensions",
            Self::Check => "check",
            Self::Package => "package",
            Self::Infobase => "infobase",
            Self::Test => "test",
            Self::Launch => "launch",
            Self::Publish => "publish",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|phase| phase.as_str() == value)
    }
}

/// Вид промежуточного сообщения агента, которое раннер передаёт как ход работы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentProgressKind {
    /// `progress` агента: сколько сделано.
    Progress,
    /// `log` агента: строка журнала.
    Log,
    /// `loading-issue` агента: замечание к загружаемому объекту.
    LoadingIssue,
}

impl AgentProgressKind {
    pub const ALL: [Self; 3] = [Self::Progress, Self::Log, Self::LoadingIssue];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Log => "log",
            Self::LoadingIssue => "loading_issue",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Почему команду отменили.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    /// Прерывание оператора: `Ctrl+C`, `SIGTERM`, `CTRL_BREAK`.
    Cancelled,
    /// Истёк срок команды. У команды командной строки срока нет
    /// (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`), и раннер этой причиной пока не отвечает.
    #[allow(
        dead_code,
        reason = "форма событий называет причину заранее; командная строка срока не имеет"
    )]
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Строка журнала и имя на проводе — одно и то же: иначе фаза, прошедшая через журнал,
    /// разбиралась бы не в ту.
    #[test]
    fn a_progress_name_is_its_wire_name() {
        let wire = |value: serde_json::Value| value.as_str().map(str::to_owned);
        for phase in ProgressPhase::ALL {
            assert_eq!(
                wire(serde_json::to_value(phase).unwrap()).as_deref(),
                Some(phase.as_str())
            );
            assert_eq!(ProgressPhase::parse(phase.as_str()), Some(phase));
        }
        for kind in AgentProgressKind::ALL {
            assert_eq!(
                wire(serde_json::to_value(kind).unwrap()).as_deref(),
                Some(kind.as_str())
            );
            assert_eq!(AgentProgressKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(
            serde_json::to_value(CancelReason::TimedOut).unwrap(),
            "timed_out"
        );
        assert_eq!(ProgressPhase::parse("other"), None);
    }
}
