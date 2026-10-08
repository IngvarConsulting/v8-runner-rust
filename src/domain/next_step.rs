//! Следующий шаг, который называет отказ.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Выходы рабочей копии к своей базе — одним текстом для каждого, кто их называет: отказа
/// копии без объявленной базы (`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`) и
/// предупреждения о базе другой копии
/// (`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`). `source` — база,
/// копию которой предлагают: `<infobase>` или имя секции, например `upstream`. Общую базу
/// выходом текст не называет: общих баз нет (#437).
pub fn ways_to_an_own_infobase(source: &str) -> String {
    format!(
        "its own clean infobase built from the sources — `v8-runner init --infobase <connection string>`, then `v8-runner infobase create`; \
         a copy of an infobase with its data — `v8-runner init --infobase <connection string>`, then `v8-runner infobase create --from {source}`; \
         an infobase deployed from a reference image — `v8-runner init --infobase <connection string>`, then `v8-runner infobase restore --input <reference>.dt --create`"
    )
}

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
