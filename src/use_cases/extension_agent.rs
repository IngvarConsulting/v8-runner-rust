//! Расширения через агентский shell Конфигуратора: группа `config extensions`.
//!
//! Ответ `properties get` двух форм (живые ответы 8.3.27.2074 от 15.09.2026):
//! `--all-extensions` — одно сообщение `success`, в `body` которого массив записей
//! `{type: "extension-properties", body: {…}}`; `--extension=X` — сообщение
//! `extension-properties` верхнего уровня с записью в `body`. Значения — JSON-логические;
//! пустая строка в `version` и `security-profile-name` означает «не задано».

use std::path::Path;

use serde_json::Value;

use crate::config::model::AppConfig;
use crate::domain::extensions::InstalledExtension;
use crate::platform::agent::{AgentMessageType, AgentReply, WaitPolicy};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    argument, connect, run_command, transcript_log, wait_policy, AgentHandle,
};
use crate::use_cases::context::ExecutionContext;

pub(crate) struct ExtensionAgent {
    handle: AgentHandle,
    wait: WaitPolicy,
}

impl ExtensionAgent {
    pub(crate) fn open(
        context: &ExecutionContext,
        config: &AppConfig,
        v8: Option<&Path>,
    ) -> Result<Self, AppError> {
        let wait = wait_policy(context);
        let log = transcript_log(config, "extensions")?;
        let mut utilities = PlatformUtilities::from_config(config);
        let handle = connect(config, &mut utilities, v8, log, &wait)?;
        Ok(Self { handle, wait })
    }

    /// `properties get --all-extensions | --extension=<name>`.
    pub(crate) fn inventory(
        &mut self,
        name: Option<&str>,
    ) -> Result<Vec<InstalledExtension>, AppError> {
        let command = match name {
            Some(name) => format!(
                "config extensions properties get --extension={}",
                argument(name)
            ),
            None => "config extensions properties get --all-extensions".to_owned(),
        };
        let reply = run_command(&mut self.handle, &command, &self.wait)?;
        parse_properties(&reply)
    }

    /// `properties set --safe-mode=no --unsafe-action-protection=no`.
    pub(crate) fn disable_safety(&mut self, name: &str) -> Result<(), AppError> {
        self.run(&format!(
            "config extensions properties set --extension={} --safe-mode=no --unsafe-action-protection=no",
            argument(name)
        ))
    }

    pub(crate) fn set_active(&mut self, name: &str, active: bool) -> Result<(), AppError> {
        self.run(&format!(
            "config extensions properties set --extension={} --active={}",
            argument(name),
            if active { "yes" } else { "no" }
        ))
    }

    /// Агент требует синоним в форме `NStr()` и не принимает ни пустой, ни простую строку
    /// (замер 15.09.2026: `--synonym="ru='X'; en='X'"` проходит, `--synonym=X` — нет).
    /// Простой синоним заворачивается в обе поставляемые локали; без синонима им
    /// становится имя.
    pub(crate) fn create(
        &mut self,
        name: &str,
        name_prefix: &str,
        synonym: Option<&str>,
        purpose: Option<&str>,
    ) -> Result<(), AppError> {
        let mut command = format!(
            "config extensions create --extension={} --name-prefix={} --synonym={}",
            argument(name),
            argument(name_prefix),
            argument(&nstr_synonym(synonym.unwrap_or(name)))
        );
        if let Some(purpose) = purpose {
            command.push_str(&format!(" --purpose={}", argument(purpose)));
        }
        self.run(&command)
    }

    pub(crate) fn delete(&mut self, name: &str) -> Result<(), AppError> {
        self.run(&format!(
            "config extensions delete --extension={}",
            argument(name)
        ))
    }

    pub(crate) fn close(self) {
        self.handle.finish(&self.wait);
    }

    fn run(&mut self, command: &str) -> Result<(), AppError> {
        run_command(&mut self.handle, command, &self.wait).map(|_| ())
    }
}

/// Уже многоязычная строка (`ru='…'`) передаётся как есть, простая — заворачивается.
fn nstr_synonym(value: &str) -> String {
    if value.contains("='") {
        value.to_owned()
    } else {
        let escaped = value.replace('\'', "''");
        format!("ru='{escaped}'; en='{escaped}'")
    }
}

fn parse_properties(reply: &AgentReply) -> Result<Vec<InstalledExtension>, AppError> {
    let mut entries: Vec<&Value> = Vec::new();
    for message in &reply.messages {
        match (message.kind, message.body.as_ref()) {
            (AgentMessageType::ExtensionProperties, Some(body)) => entries.push(body),
            (AgentMessageType::Success, Some(Value::Array(items))) => entries.extend(items),
            (AgentMessageType::Success, Some(other)) if !other.is_null() => {
                return Err(AppError::InvalidOutput(
                    "agent extension properties reply body is not an array".to_owned(),
                ));
            }
            _ => {}
        }
    }
    entries
        .iter()
        .map(|entry| {
            let record = entry.get("body").unwrap_or(entry);
            let text = |key: &str| -> Result<String, AppError> {
                record
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        AppError::InvalidOutput(format!(
                            "agent extension properties record lacks '{key}'"
                        ))
                    })
            };
            let flag = |key: &str| -> Result<bool, AppError> {
                match record.get(key) {
                    Some(Value::Bool(value)) => Ok(*value),
                    Some(Value::String(value)) if value == "yes" || value == "true" => Ok(true),
                    Some(Value::String(value)) if value == "no" || value == "false" => Ok(false),
                    _ => Err(AppError::InvalidOutput(format!(
                        "agent extension properties record lacks a boolean '{key}'"
                    ))),
                }
            };
            let optional = |key: &str| -> Result<Option<String>, AppError> {
                text(key).map(|value| if value.is_empty() { None } else { Some(value) })
            };
            Ok(InstalledExtension {
                name: text("name")?,
                version: optional("version")?,
                active: flag("active")?,
                purpose: text("purpose")?,
                safe_mode: flag("safe-mode")?,
                security_profile_name: optional("security-profile-name")?,
                unsafe_action_protection: flag("unsafe-action-protection")?,
                used_in_distributed_infobase: flag("used-in-distributed-infobase")?,
                scope: text("scope")?,
                hash_sum: text("hash-sum")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{nstr_synonym, parse_properties};

    #[test]
    fn a_plain_synonym_is_wrapped_and_an_nstr_one_is_kept() {
        assert_eq!(nstr_synonym("Проба"), "ru='Проба'; en='Проба'");
        assert_eq!(nstr_synonym("ru='Проба'"), "ru='Проба'");
    }

    fn reply(json: &str) -> crate::platform::agent::AgentReply {
        crate::platform::agent::AgentReply {
            messages: serde_json::from_str(json).expect("messages"),
        }
    }

    /// `--all-extensions`: записи вложены в `body` сообщения `success`.
    #[test]
    fn a_live_shaped_list_reply_is_read_as_records() {
        let records = parse_properties(&reply(
            r#"[{"type":"success","message":"","body":[{"type":"extension-properties","body":{"name":"Зонд","version":"","active":true,"purpose":"customization","safe-mode":true,"security-profile-name":"","unsafe-action-protection":true,"used-in-distributed-infobase":false,"scope":"infobase","hash-sum":"lp1b"}}]}]"#,
        ))
        .expect("records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "Зонд");
        assert_eq!(records[0].version, None);
        assert!(records[0].active);
        assert!(records[0].safe_mode);
        assert_eq!(records[0].hash_sum, "lp1b");
    }

    /// `--extension=X`: запись — сообщение `extension-properties` верхнего уровня.
    #[test]
    fn a_live_shaped_single_reply_is_read_as_a_record() {
        let records = parse_properties(&reply(
            r#"[{"type":"extension-properties","body":{"name":"Зонд","version":"1.0","active":false,"purpose":"customization","safe-mode":true,"security-profile-name":"","unsafe-action-protection":true,"used-in-distributed-infobase":false,"scope":"infobase","hash-sum":"lp1b"}},{"type":"success","message":"","body":[]}]"#,
        ))
        .expect("records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].version.as_deref(), Some("1.0"));
        assert!(!records[0].active);
    }

    #[test]
    fn a_record_without_a_flag_is_refused_not_defaulted() {
        assert!(parse_properties(&reply(
            r#"[{"type":"success","message":"","body":[{"type":"extension-properties","body":{"name":"Зонд","version":"","purpose":"customization","safe-mode":true,"security-profile-name":"","unsafe-action-protection":true,"used-in-distributed-infobase":false,"scope":"infobase","hash-sum":"x"}}]}]"#,
        ))
        .is_err());
    }
}
