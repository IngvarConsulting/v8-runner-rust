//! Следующий шаг, который называет отказ.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Шаг, которым вызывающий выходит из отказа: команда, при нужде набор исходников и ключи.
///
/// Поле отвечает машине; человеку остаётся текст сообщения, и он не сокращается.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NextStep {
    /// Имя команды словаря, как его печатает конверт: `launch web`, `pull`, `init`.
    pub command: String,
    /// Набор исходников, когда шаг относится к одному набору.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
    /// Ключи команды со значениями: `--mode` → `combine`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub keys: BTreeMap<String, String>,
}

impl NextStep {
    /// Шаг из одной команды.
    pub fn command(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            source_set: None,
            keys: BTreeMap::new(),
        }
    }

    /// Шаг для одного набора исходников.
    pub fn for_source_set(mut self, source_set: impl Into<String>) -> Self {
        self.source_set = Some(source_set.into());
        self
    }

    /// Добавляет ключ команды со значением; у ключа без значения оно пустое.
    pub fn with_key(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.keys.insert(key.into(), value.into());
        self
    }
}
