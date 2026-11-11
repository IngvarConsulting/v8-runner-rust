//! Ход команды, пока она идёт: этапы живой ленты, сообщения агента и отмена.
//!
//! Сценарий о выводе не знает: всё уходит событием журнала с целью
//! `v8_runner::live_progress`, а журнал рисует его строкой живой ленты в тексте и событием
//! потока `--events -` при ответе JSON (`INV.USE-CASES.A-LONG-AGENT-COMMAND-SHOWS-ITS-PROGRESS`).

use tracing::info;

use crate::domain::progress::{AgentProgressKind, ProgressPhase};
use crate::platform::agent::{AgentMessage, AgentMessageType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveStageStatus {
    Succeeded,
    Failed,
}

impl LiveStageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// Emits a text-mode live progress timeline event before a blocking stage starts; the same
/// event is a `step` of the `--events -` stream with its `phase` and source `set`.
///
/// Callers must pass fixed, sanitized vocabulary only: no rendered commands, raw
/// arguments, stdout/stderr, environment values, connection strings, or secrets.
pub(crate) fn log_live_stage(phase: ProgressPhase, set: Option<&str>, label: &str, detail: &str) {
    info!(
        target: "v8_runner::live_progress",
        timeline_status = "running",
        timeline_label = label,
        timeline_detail = detail,
        progress_phase = phase.as_str(),
        progress_set = set
    );
}

/// Emits a sanitized live progress status after a blocking stage finishes.
pub(crate) fn log_live_stage_status(label: &str, status: LiveStageStatus, detail: &str) {
    info!(
        target: "v8_runner::live_progress",
        timeline_status = status.as_str(),
        timeline_label = label,
        timeline_detail = detail
    );
}

/// Промежуточные сообщения одного массива ответа агента — ход команды. Текст проходит маску
/// известных учётных данных (`secrets`) и словаря секретов прозы
/// (`crate::platform::secrets::mask_text`).
pub(crate) fn log_agent_messages(batch: &[AgentMessage], secrets: &[&str]) {
    for message in batch {
        let Some(kind) = progress_kind(message.kind) else {
            continue;
        };
        let Some(text) = message_text(message) else {
            continue;
        };
        let text = without_control_chars(&crate::platform::secrets::mask_text(&text, secrets));
        if text.trim().is_empty() {
            continue;
        }
        info!(
            target: "v8_runner::live_progress",
            agent_type = kind.as_str(),
            agent_text = text.as_str()
        );
    }
}

/// Промежуточные сообщения — ход; итог, вопрос и уведомления ходом не являются.
fn progress_kind(kind: AgentMessageType) -> Option<AgentProgressKind> {
    match kind {
        AgentMessageType::Progress => Some(AgentProgressKind::Progress),
        AgentMessageType::Log => Some(AgentProgressKind::Log),
        AgentMessageType::LoadingIssue => Some(AgentProgressKind::LoadingIssue),
        AgentMessageType::Success
        | AgentMessageType::Error
        | AgentMessageType::Canceled
        | AgentMessageType::Question
        | AgentMessageType::Dbstru
        | AgentMessageType::ExtensionInfo
        | AgentMessageType::ExtensionProperties
        | AgentMessageType::GenerationId => None,
    }
}

/// Текст сообщения: `message`, а у `progress` без него — `message` и `percent` из `body`
/// (`{"message": …, "percent": …}`) или строка `body`. Тело другого вида ходом не печатается.
fn message_text(message: &AgentMessage) -> Option<String> {
    if let Some(text) = message
        .message
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        return Some(text.to_owned());
    }
    match message.body.as_ref()? {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(body) => {
            let text = body.get("message").and_then(serde_json::Value::as_str);
            let percent = body.get("percent").filter(|percent| percent.is_number());
            match (text, percent) {
                (Some(text), Some(percent)) => Some(format!("{text} {percent}%")),
                (Some(text), None) => Some(text.to_owned()),
                (None, Some(percent)) => Some(format!("{percent}%")),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

/// Управляющие знаки текста агента — кроме перевода строки — становятся пробелами: чужая
/// последовательность ESC не ломает строку хода в терминале.
fn without_control_chars(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_control() && ch != '\n' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(raw: &str) -> AgentMessage {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn the_text_of_a_progress_message_is_its_message_or_its_body() {
        assert_eq!(
            message_text(&message(r#"{"type":"log","message":"Загрузка"}"#)).as_deref(),
            Some("Загрузка")
        );
        assert_eq!(
            message_text(&message(
                r#"{"type":"progress","body":{"message":"dump","percent":10}}"#
            ))
            .as_deref(),
            Some("dump 10%")
        );
        assert_eq!(
            message_text(&message(r#"{"type":"progress","body":{"percent":50}}"#)).as_deref(),
            Some("50%")
        );
        assert_eq!(message_text(&message(r#"{"type":"progress"}"#)), None);
        assert_eq!(
            message_text(&message(r#"{"type":"progress","body":{"stage":1}}"#)),
            None
        );
        assert_eq!(without_control_chars("a\u{1b}[31mb\nc"), "a [31mb\nc");
        assert_eq!(progress_kind(AgentMessageType::Success), None);
        assert_eq!(
            progress_kind(AgentMessageType::LoadingIssue),
            Some(AgentProgressKind::LoadingIssue)
        );
    }
}
