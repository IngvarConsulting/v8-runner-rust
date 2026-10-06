use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Сообщение удачной выгрузки, которой нечего сообщить.
///
/// Его же рендерер отличает от настоящего предупреждения, поэтому фраза живёт одним
/// значением: разъехавшись, они сделали бы безоблачную выгрузку предупреждением.
pub const DUMP_SUCCESS_MESSAGE: &str = "dump completed successfully";

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DumpResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// Получил ли исполнитель работу этой команды: запущен процесс, который её выполняет,
    /// либо работающей сессии отдана команда запроса. Подъём и открытие сессии, в том числе
    /// запуск её процесса, и её служебные команды работой не считаются. `false`, если работы
    /// не было: превью, отказ или прерывание до передачи работы, процесс, который не удалось
    /// запустить. Поле есть всегда, поэтому отсутствие работы не выводится из отсутствия
    /// значения.
    pub provider_dispatched: bool,
    /// `true` when the platform reported the configuration generation unchanged since the
    /// last recorded build or dump and nothing was dumped.
    #[serde(default)]
    pub up_to_date: bool,
    pub source_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectors: Option<Vec<DumpSelectorResult>>,
    pub mode: DumpMode,
    pub target_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Что в каталоге набора пропадает безвозвратно — каждый путь: после выгрузки с
    /// согласием (`--force`) — уничтоженное, у превью — что выгрузка уничтожила бы или на
    /// чём остановилась бы без согласия. Пути гит называет от корня рабочей копии; там, где
    /// он не ответил, путь полный и потерей считается каждый файл каталога. Поля нет, когда
    /// терять нечего.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DumpSelectorResult {
    pub requested: String,
    pub normalized: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DumpMode {
    Full,
    Incremental,
    Partial,
}

/// Ответ `pull --all`: наборы по составу базы.
///
/// Каждая выгрузка отчитывается формой `pull <SET>` в порядке обхода; объявленные этой
/// командой наборы названы отдельно, теми же полями, какими их записал `init`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PullAllResult {
    /// Квитанция о выборе исполнителя, который читал состав базы и выгружал наборы;
    /// `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,
    pub ok: bool,
    /// Получил ли исполнитель работу этой команды — то же, что у `pull <SET>`.
    pub provider_dispatched: bool,
    /// Наборы, которые команда объявила в `v8project.yaml` для расширений базы без набора,
    /// в порядке объявления. `null` — состав базы не читали: у превью и у отказа до чтения.
    #[schemars(required, extend("type" = ["array", "null"]))]
    pub declared: Option<Vec<crate::domain::config_init::ConfigInitSourceSet>>,
    /// Наборы расширений проекта, которых в базе нет: их не выгружали. Поля нет, когда
    /// таких нет или состав не читали.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_installed: Vec<String>,
    /// Расширения базы без набора, которым набор не объявлен, с причиной: объявление
    /// невозможно, а прочие наборы выгружаются. Поля нет, когда таких нет.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_declared: Vec<NotDeclaredExtension>,
    /// Только у превью: наборы расширений проекта, которые превью не выгружало, потому что
    /// состава базы не знает. Настоящий прогон выгрузит те из них, чьё расширение в базе
    /// есть, а прочие назовёт в `not_installed`. Поля нет, когда таких наборов нет.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub if_installed: Vec<String>,
    /// Выгрузка каждого набора формой `pull <SET>` в порядке обхода: сперва наборы проекта,
    /// затем объявленные. После первого отказа обход останавливается. У превью — только
    /// основная конфигурация: её прогон выгрузит при любом составе базы.
    pub sets: Vec<DumpResult>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Расширение базы, которому `pull --all` не объявил набор, и почему.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NotDeclaredExtension {
    /// Имя расширения в базе.
    pub name: String,
    /// Почему набор не объявлен и что сделать вместо этого.
    pub reason: String,
}
