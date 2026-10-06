//! Состояние пары «каталог ↔ база»: ответ `status`, `status --all` и `status --deep`.
//!
//! Без `--deep` ответ собран из памяти под `workPath` и платформу не запускает
//! (`INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM`); с `--deep` к нему добавляется то, что
//! ответила платформа: поколение каждого набора, состав расширений и копии, которые держат
//! файловую базу. Форма закреплена `CTR.WIRE.STATUS-DATA`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::capability::{Provider, ProviderReceipt, TargetKind};

/// Ответ `status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct StatusResult {
    /// Спрашивали ли платформу (`--deep`). `false` — ответ собран из памяти под `workPath`.
    pub deep: bool,
    /// Базы ответа: выбранная или, у `--all`, все объявленные в местном слое по имени.
    pub infobases: Vec<InfobaseStatus>,
    pub duration_ms: u64,
}

/// Состояние одной базы.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InfobaseStatus {
    /// Имя базы в местном слое; `null` — база пришла строкой соединения в `--infobase`.
    pub name: Option<String>,
    /// Та база, с которой работает команда без `--infobase` или с ним.
    pub selected: bool,
    pub kind: TargetKind,
    /// Адрес базы без учётных данных — тот, к которому привязана память; `null` — адрес не
    /// распознан, и памяти о базе у копии быть не может.
    pub address: Option<String>,
    /// Копия взяла базу без метки или сменила ушедшего владельца и с тех пор не отправляла
    /// в неё: когда. `null` — признака нет.
    pub new_owner_since: Option<String>,
    /// Наборы исходников, которые ходят в базу: конфигурация и расширения.
    pub source_sets: Vec<SourceSetStatus>,
    /// Состав расширений базы рядом с наборами проекта; только у `--deep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ExtensionsStatus>,
    /// Копии, которые держат файловую базу, по её метке; только у `--deep` и только у
    /// файловой базы.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holders: Option<HoldersStatus>,
}

/// Состояние набора исходников относительно базы.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SourceSetStatus {
    pub name: String,
    /// `configuration` или `extension`.
    pub purpose: String,
    /// Помнит ли копия базу для этого набора.
    pub memory: MemoryState,
    /// Поколение, записанное после последнего обмена; `null` — записи этой пары нет.
    pub recorded: Option<RecordedGeneration>,
    /// Файлы каталога, изменившиеся с тех пор, как раннер его последний раз читал; `null` —
    /// сравнить не с чем.
    pub changed_files: Option<u64>,
    /// Что о поколении ответила платформа; только у `--deep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<BaseGeneration>,
}

/// Память о базе у набора (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryState {
    /// Своя запись поколения или своя хеш-память этой пары: `push` идёт без отказа `no_memory`.
    Remembered,
    /// Памяти нет: `push` откажет `no_memory`.
    #[serde(rename = "none")]
    Missing,
    /// Под именем базы лежит память другой пары «база ↔ каталог»; она не используется.
    Foreign,
    /// Хеш-память не открывается; она не используется.
    Unreadable,
    /// Адрес базы не распознан: помнить её нельзя.
    Unbound,
}

/// Запись журнала поколений.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct RecordedGeneration {
    pub token: String,
    /// Инструмент, которым токен получен: токены разных инструментов несравнимы.
    pub tool: Provider,
    /// `build`, `dump` или `failed_build` — после чего записан токен.
    pub after: String,
    pub recorded_at: String,
}

/// Поколение, которое ответила платформа, и его сверка с записью.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct BaseGeneration {
    /// Инструмент, которым спрошено поколение: исполнитель `push` для этой базы. `null` —
    /// готового исполнителя нет.
    pub tool: Option<Provider>,
    /// Ответ; `null` — ответа нет.
    pub token: Option<String>,
    pub comparison: GenerationVerdict,
    /// Почему ответа нет, если его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Сверка ответа с записью — та же, что сделает `push` перед загрузкой.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GenerationVerdict {
    /// Тот же инструмент отдал записанный токен: базу с последнего обмена не меняли.
    Unchanged,
    /// Тот же инструмент отдал другой токен: база ушла вперёд, `push` откажет
    /// `non_fast_forward`.
    MovedAhead,
    /// Запись сделана другим инструментом: сравнивать не с чем.
    OtherTool,
    /// Записи этой пары нет.
    NoRecord,
    /// Платформа поколением не ответила.
    NoAnswer,
}

/// Состав расширений базы рядом с наборами проекта.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExtensionsStatus {
    /// Квитанция о выборе исполнителя; `null` — выбор не начинался.
    pub provider: Option<ProviderReceipt>,
    /// Расширения базы; `null` — состав не прочитан, причина в `reason`.
    pub installed: Option<Vec<InstalledExtensionStatus>>,
    /// Наборы расширений проекта, которых в базе нет; `null` — состав не прочитан.
    pub missing_in_base: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Расширение, установленное в базе.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InstalledExtensionStatus {
    /// Имя расширения в базе.
    pub name: String,
    pub active: bool,
    /// Набор проекта с тем же именем; `null` — есть в базе, нет в проекте.
    pub source_set: Option<String>,
}

/// Копии, которые держат файловую базу.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct HoldersStatus {
    /// Файл метки рядом с каталогом базы.
    pub marker: PathBuf,
    /// Записи метки; `null` — метку не прочитать, причина в `reason`. Пустой список — базу
    /// никто не держит.
    pub owners: Option<Vec<HolderStatus>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Копия из метки владельца.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct HolderStatus {
    /// Каталог проекта копии.
    pub project: PathBuf,
    /// Имя хоста на момент записи.
    pub host: Option<String>,
    /// Согласие копии делить базу, записанное в метке.
    pub shared: bool,
    #[schemars(with = "String", extend("format" = "date-time"))]
    pub since: DateTime<Utc>,
    /// Это та копия, из которой спрашивают.
    pub this_copy: bool,
}
