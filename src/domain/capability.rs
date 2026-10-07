//! Матрица возможностей: кто исполняет операцию на цели данного вида.
//!
//! Единственный источник для трёх потребителей — валидации конфига, выбора исполнителя
//! перед запуском и таблицы возможностей в документации. Строка матрицы — данные, а не
//! код: порядок в цепочке назначает владелец проекта, и правка порядка — изменение
//! поведения, видимое в квитанции.
//!
//! На этом шаге цепочки повторяют вчерашний выбор по ключу `builder`: первым стоит тот,
//! кого раннер брал по умолчанию, вторым — тот, кого можно было назначить ключом. Замер
//! каждой строки записан как улика и воротами не является.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Исполнитель. Имя называет того, кто делает работу, а не способ его запуска.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Конфигуратор пакетным процессом на операцию.
    Designer,
    /// Агентский shell: Конфигуратор в агентском режиме или шлюз автономного сервера.
    Agent,
    /// Утилита `ibcmd`.
    Ibcmd,
    /// Конвертер XML ↔ CF без платформы.
    IbcmdRs,
    /// Публикация базы на веб-сервере.
    Webinst,
}

impl Provider {
    // Полный перечень и разбор по имени держат тесты словаря; продуктовый путь
    // получает имена через serde и в них не ходит.
    #[allow(dead_code)]
    pub const ALL: [Self; 5] = [
        Self::Designer,
        Self::Agent,
        Self::Ibcmd,
        Self::IbcmdRs,
        Self::Webinst,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Designer => "designer",
            Self::Agent => "agent",
            Self::Ibcmd => "ibcmd",
            Self::IbcmdRs => "ibcmd-rs",
            Self::Webinst => "webinst",
        }
    }

    #[allow(dead_code)]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.as_str() == value)
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Операция, у которой есть строка в матрице.
///
/// Здесь только то, что идёт к платформе и может идти к ней разными исполнителями.
/// `launch`, прогон тестов клиентом и `bootstrap` строк не имеют: у них один инструмент, и
/// назначать им исполнителя нечего. У `convert` строка — у направлений с пакетом; перевод
/// между EDT и XML делает `1cedtcli`, и строки у него нет.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum Operation {
    #[serde(rename = "infobase.create", alias = "init")]
    Init,
    #[serde(rename = "push", alias = "build")]
    Build,
    #[serde(rename = "upload", alias = "load")]
    Load,
    #[serde(rename = "pull", alias = "dump")]
    Dump,
    #[serde(rename = "extensions")]
    Extensions,
    #[serde(rename = "download", alias = "infobase.configuration.export")]
    ConfigurationExport,
    #[serde(rename = "infobase.dump")]
    InfobaseDump,
    #[serde(rename = "infobase.restore")]
    InfobaseRestore,
    #[serde(rename = "syntax")]
    Syntax,
    #[serde(rename = "make")]
    Make,
    #[serde(rename = "convert")]
    Convert,
    #[serde(rename = "publish")]
    Publish,
}

impl Operation {
    pub const ALL: [Self; 12] = [
        Self::Init,
        Self::Build,
        Self::Load,
        Self::Dump,
        Self::Extensions,
        Self::ConfigurationExport,
        Self::InfobaseDump,
        Self::InfobaseRestore,
        Self::Syntax,
        Self::Make,
        Self::Convert,
        Self::Publish,
    ];

    /// Ключ в `providers:` и имя в квитанции.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Init => "infobase.create",
            Self::Build => "push",
            Self::Load => "upload",
            Self::Dump => "pull",
            Self::Extensions => "extensions",
            Self::ConfigurationExport => "download",
            Self::InfobaseDump => "infobase.dump",
            Self::InfobaseRestore => "infobase.restore",
            Self::Syntax => "syntax",
            Self::Make => "make",
            Self::Convert => "convert",
            Self::Publish => "publish",
        }
    }

    /// Прежнее имя ключа `providers.*`, принимаемое один цикл выпуска. Один владелец
    /// списка синонимов: по нему и разбирают ключ, и объясняют переименование.
    pub const fn previous_key(self) -> Option<&'static str> {
        match self {
            Self::Init => Some("init"),
            Self::Build => Some("build"),
            Self::Load => Some("load"),
            Self::Dump => Some("dump"),
            Self::ConfigurationExport => Some("infobase.configuration.export"),
            _ => None,
        }
    }

    /// Ключ конфигурации: имя команды или прежнее имя того же ключа.
    pub fn parse_config_key(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|operation| {
            operation.as_str() == value || operation.previous_key() == Some(value)
        })
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.as_str() == value)
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Вид информационной базы — вторая координата матрицы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    File,
    Cluster,
    Standalone,
}

impl TargetKind {
    // Полный перечень держат тесты матрицы и её артефакт; продуктовый путь получает вид
    // цели из конфигурации и в перечень не ходит.
    #[allow(dead_code)]
    pub const ALL: [Self; 3] = [Self::File, Self::Cluster, Self::Standalone];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Cluster => "cluster",
            Self::Standalone => "standalone",
        }
    }
}

/// Насколько исполнитель реализует операцию.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Implementation {
    /// В цепочке умолчаний.
    Implemented,
    /// Реализован, но в умолчания не входит: назначается только переопределением.
    Experimental,
}

/// Чем доказана строка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Documented,
    ArgvTested,
    LiveVerified,
}

/// Одна возможность: исполнитель, реализующий операцию на цели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub provider: Provider,
    pub implementation: Implementation,
    pub evidence: Evidence,
}

const fn implemented(provider: Provider, evidence: Evidence) -> Capability {
    Capability {
        provider,
        implementation: Implementation::Implemented,
        evidence,
    }
}

const fn experimental(provider: Provider, evidence: Evidence) -> Capability {
    Capability {
        provider,
        implementation: Implementation::Experimental,
        evidence,
    }
}

/// Строки матрицы в порядке умолчаний.
///
/// Первый реализованный исполнитель в строке — умолчание; остальные реализованные
/// пробуются, если он не готов; экспериментальные назначаются только переопределением.
pub fn capabilities(operation: Operation, target: TargetKind) -> &'static [Capability] {
    use Evidence::{ArgvTested, Documented, LiveVerified};
    use Provider::{Agent, Designer, Ibcmd, Webinst};

    // Публикация на веб-сервере: у операции нет развилки, только `webinst`.
    const WEBINST_ONLY: &[Capability] = &[implemented(Webinst, Documented)];

    // Создание файловой базы: `ibcmd infobase create --import --apply --force` собирает
    // базу сразу с основной конфигурацией (замер 8.3.27.2074 у прямого шлюза #205);
    // Конфигуратор — запасной: `CREATEINFOBASE`, затем `/LoadConfigFromFiles` и
    // `/UpdateDBCfg`. Порядок назначил владелец (#204).
    const CREATE_FILE: &[Capability] = &[
        implemented(Ibcmd, ArgvTested),
        implemented(Designer, LiveVerified),
    ];
    // Создание базы в кластере: Конфигуратор `CREATEINFOBASE` с клиент-серверной строкой
    // регистрирует базу и создаёт базу данных одной командой (замер #181, 06.10.2026,
    // 8.5.4.1878). `ibcmd` о кластере не знает и строки не имеет; запасной путь `rac` — не
    // исполнитель матрицы и ещё не реализован (#213).
    const CREATE_CLUSTER: &[Capability] = &[implemented(Designer, ArgvTested)];
    // Агент назначается только ключом `providers.<op>: agent`: его место в цепочке
    // умолчаний назначает владелец. Путь раннера через агента прогнан вживую
    // 15.09.2026 на 8.3.27.2074: полная и частичная загрузка с `update-db-cfg` в одной
    // сессии, полная выгрузка через staging, короткое замыкание по поколению.
    const DUMP: &[Capability] = &[
        implemented(Designer, LiveVerified),
        implemented(Ibcmd, ArgvTested),
        experimental(Agent, LiveVerified),
    ];
    const BUILD: &[Capability] = &[
        implemented(Designer, LiveVerified),
        implemented(Ibcmd, ArgvTested),
        experimental(Agent, LiveVerified),
    ];
    const DESIGNER_ONLY: &[Capability] = &[implemented(Designer, LiveVerified)];
    // Шлюз прогнан раннером на живом `ibsrv` 8.3.27 15.09.2026: build (полная и частичная
    // загрузка), dump (полная и пропуск по поколению), make cf, export cf, extensions.
    const GATE_ONLY: &[Capability] = &[implemented(Agent, LiveVerified)];
    // Автономный сервер: Конфигуратор первым — по прямому шлюзу, как в кластер, агент
    // вторым — по SSH-шлюзу. Команды Конфигуратора через прямой шлюз замерены вручную
    // (#178, #179, 06.10.2026, 8.3.27.2074): `/LoadConfigFromFiles`, `/DumpConfigToFiles`,
    // `/UpdateDBCfg`, `/CheckConfig`, `/CompareCfg`, `/DumpDBCfg`, `/DumpIB`, `/RestoreIB`;
    // путь раннера проверен по командной строке. Кому из них путь объявлен, решает
    // конфигурация (`AppConfig::missing_way`), а не строка матрицы.
    const DIRECT_GATE_THEN_GATE: &[Capability] = &[
        implemented(Designer, ArgvTested),
        implemented(Agent, LiveVerified),
    ];
    const DIRECT_GATE_ONLY: &[Capability] = &[implemented(Designer, ArgvTested)];
    // `make` собирает пакет из исходников во временной базе раннера, а не выгружает базу
    // проекта, поэтому строка от вида цели не зависит. `ibcmd`: `infobase create`, затем
    // `config import --out` (замер #182, 06.10.2026, 8.3.27.2074); Конфигуратор:
    // `CREATEINFOBASE`, `/LoadConfigFromFiles`, `/DumpCfg` (тот же замер). Цепочку назначил
    // владелец (#364). `ibcmd-rs` строки не имеет до замера #413, агент снят: ему нужна база
    // проекта.
    const MAKE: &[Capability] = &[
        implemented(Ibcmd, ArgvTested),
        implemented(Designer, ArgvTested),
    ];
    // `convert` с пакетом базы проекта не открывает: `ibcmd` работает во временной базе
    // раннера, поэтому строка от вида цели не зависит. XML → пакет — `config import --out`
    // (замер #182); пакет → XML — `config export --file` (вызов взят у `extensions`, живой
    // замер во временной базе — #416). `ibcmd-rs` идёт за `ibcmd` после замера #413, до него
    // строки не имеет.
    const CONVERT: &[Capability] = &[implemented(Ibcmd, ArgvTested)];
    // Агент: `config extensions …` — list/info/create/activate/delete и снятие защиты
    // прогнаны раннером на 8.3.27 15.09.2026.
    const EXTENSIONS: &[Capability] = &[
        implemented(Ibcmd, LiveVerified),
        experimental(Agent, LiveVerified),
    ];
    // Агент: `config dump-cfg` для рабочей конфигурации прогнан раннером 15.09.2026.
    const EXPORT: &[Capability] = &[
        implemented(Designer, ArgvTested),
        implemented(Ibcmd, ArgvTested),
        experimental(Agent, LiveVerified),
    ];
    // Агент: `infobase-tools dump-ib` и `restore-ib` (с обрывом сессии после загрузки)
    // прогнаны раннером 15.09.2026.
    const SNAPSHOT: &[Capability] = &[
        implemented(Designer, ArgvTested),
        experimental(Ibcmd, Documented),
        experimental(Agent, LiveVerified),
    ];

    match (operation, target) {
        (Operation::Init, TargetKind::File) => CREATE_FILE,
        (Operation::Init, TargetKind::Cluster) => CREATE_CLUSTER,
        (Operation::Build, TargetKind::File | TargetKind::Cluster) => BUILD,
        (Operation::Dump, TargetKind::File | TargetKind::Cluster) => DUMP,
        (Operation::Load | Operation::Syntax, TargetKind::File | TargetKind::Cluster) => {
            DESIGNER_ONLY
        }
        (Operation::Make, _) => MAKE,
        (Operation::Convert, _) => CONVERT,
        (Operation::Extensions, TargetKind::File | TargetKind::Cluster) => EXTENSIONS,
        (Operation::ConfigurationExport, TargetKind::File | TargetKind::Cluster) => EXPORT,
        (
            Operation::InfobaseDump | Operation::InfobaseRestore,
            TargetKind::File | TargetKind::Cluster,
        ) => SNAPSHOT,
        (Operation::Publish, TargetKind::File | TargetKind::Cluster) => WEBINST_ONLY,
        // Автономный сервер раннер не запускает и не создаёт, поэтому `init` и `publish`
        // строк не имеют: базу сервера создают до его запуска, HTTP он отдаёт сам.
        // `load`, `syntax` и снимок — только Конфигуратор по прямому шлюзу: у SSH-шлюза нет
        // `compare-cfg` и `check-config`, а `infobase-tools dump-ib` через него роняет
        // `ibsrv` 8.3.27 (живой прогон 15.09.2026, #189). Состав расширений — только агент:
        // Конфигуратора для `extensions` у раннера нет ни у какой цели (#206).
        (
            Operation::Build | Operation::Dump | Operation::ConfigurationExport,
            TargetKind::Standalone,
        ) => DIRECT_GATE_THEN_GATE,
        (
            Operation::Load
            | Operation::Syntax
            | Operation::InfobaseDump
            | Operation::InfobaseRestore,
            TargetKind::Standalone,
        ) => DIRECT_GATE_ONLY,
        (Operation::Extensions, TargetKind::Standalone) => GATE_ONLY,
        (Operation::Init | Operation::Publish, TargetKind::Standalone) => &[],
    }
}

/// Цепочка умолчаний: реализованные исполнители в порядке строки.
pub fn default_chain(operation: Operation, target: TargetKind) -> Vec<Provider> {
    capabilities(operation, target)
        .iter()
        .filter(|capability| capability.implementation == Implementation::Implemented)
        .map(|capability| capability.provider)
        .collect()
}

/// Есть ли у пары «операция и цель» настоящая развилка.
///
/// Переопределение имеет смысл только там, где есть из чего выбирать: второй
/// реализованный исполнитель либо экспериментальный, которого иначе не назначить.
pub fn has_a_choice(operation: Operation, target: TargetKind) -> bool {
    capabilities(operation, target).len() > 1
}

/// Не зависит ли строка операции от вида цели: такой операции база проекта не нужна
/// (`make` и `convert` работают во временной базе раннера), и вид цели её не касается.
pub fn needs_no_target(operation: Operation) -> bool {
    TargetKind::ALL
        .into_iter()
        .all(|target| capabilities(operation, target) == capabilities(operation, TargetKind::File))
}

/// Исполнитель, которого операция больше не принимает, и выход для того, кто его назначал.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemovedProvider {
    /// Почему исполнитель снят.
    pub reason: &'static str,
    /// Команда, которая делает то, ради чего его назначали.
    pub way_out: &'static str,
}

/// Снят ли исполнитель с операции. Один владелец перечня: по нему отказывает валидация
/// конфигурации и называет выход.
pub const fn removed_provider(operation: Operation, provider: Provider) -> Option<RemovedProvider> {
    match (operation, provider) {
        // `make` собирает пакет из исходников во временной базе раннера (#364), а агент
        // работает только с базой проекта: пакет этой базы выгружает `download`.
        (Operation::Make, Provider::Agent) => Some(RemovedProvider {
            reason: "make builds the package from the sources in a throwaway infobase of the runner, and the agent works only with the project infobase",
            way_out: "download",
        }),
        _ => None,
    }
}

/// Реализует ли исполнитель операцию на цели хоть как-то.
pub fn capability_of(
    operation: Operation,
    target: TargetKind,
    provider: Provider,
) -> Option<Capability> {
    capabilities(operation, target)
        .iter()
        .copied()
        .find(|capability| capability.provider == provider)
}

/// Выгружает ли исполнитель конфигурацию базы данных (`download --state db`).
///
/// Строка `download` матрицы говорит, кто выгружает пакет вообще; состояние конфигурации
/// сужает её. Конфигуратор делает это `/DumpDBCfg`, `ibcmd` — `config save --db`; у
/// агентского shell команды для конфигурации базы данных нет, только `config dump-cfg`
/// рабочей.
pub const fn exports_database_configuration(provider: Provider) -> bool {
    match provider {
        Provider::Designer | Provider::Ibcmd => true,
        Provider::Agent | Provider::IbcmdRs | Provider::Webinst => false,
    }
}

/// Исполнители конфигурации базы данных в порядке словаря — для текста отказа.
pub fn database_configuration_exporters() -> impl Iterator<Item = Provider> {
    Provider::ALL
        .into_iter()
        .filter(|provider| exports_database_configuration(*provider))
}

/// Откуда взялся выбранный исполнитель.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderOrigin {
    /// Первый готовый из цепочки умолчаний.
    Default,
    /// Назначен ключом `providers.<операция>` в названном файле.
    Override { file: String },
}

/// Исполнитель, пропущенный до выбранного, и почему.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SkippedProvider {
    pub provider: Provider,
    pub reason: String,
}

/// Квитанция о выборе исполнителя — одна форма у всех операций.
///
/// Объясняет принятое решение и не предлагает другого: неиспользованные альтернативы
/// не перечисляются, потому что выбор вызывающему не принадлежит. `selected: null`
/// значит, что выбор состоялся и никто не подошёл: каждый пропущенный назван с причиной.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderReceipt {
    pub selected: Option<Provider>,
    pub origin: ProviderOrigin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedProvider>,
    /// Точка входа сессии агента, через которую шло исполнение. Нет — сессия не
    /// открывалась: исполнял процесс платформы, это превью или отказ до подключения.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<SessionEndpoint>,
}

/// Как раннер добрался до точки входа агента.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Агента Конфигуратора поднял сам раннер.
    Managed,
    /// Агент Конфигуратора поднят не раннером, раннер к нему подключился.
    Attached,
    /// SSH-шлюз автономного сервера.
    Gate,
}

/// Точка входа открытой сессии: режим и `host:port`, к которому раннер подключился.
///
/// Адрес строится из разобранных хоста и порта, а не из записи конфига, поэтому
/// учётных данных в нём нет: ни `user:pass@`, ни пароля строки соединения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionEndpoint {
    pub mode: SessionMode,
    pub address: String,
}

impl ProviderReceipt {
    pub fn new(selected: Provider, origin: ProviderOrigin) -> Self {
        Self {
            selected: Some(selected),
            origin,
            skipped: Vec::new(),
            endpoint: None,
        }
    }

    /// Никто не готов: пропущены все, кого пробовали.
    pub fn nobody(origin: ProviderOrigin, skipped: Vec<SkippedProvider>) -> Self {
        Self {
            selected: None,
            origin,
            skipped,
            endpoint: None,
        }
    }

    pub fn with_skipped(mut self, skipped: Vec<SkippedProvider>) -> Self {
        self.skipped = skipped;
        self
    }
}

/// Кто назначен операции до проверки готовности.
///
/// Переопределение — ровно один исполнитель и никакого отката: если он не готов,
/// операция отказывает с причиной. Умолчание — цепочка, из которой берётся первый
/// готовый.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPlan {
    Override { provider: Provider, file: String },
    Default { chain: Vec<Provider> },
}

impl ProviderPlan {
    /// Исполнители в том порядке, в котором их пробуют.
    pub fn candidates(&self) -> Vec<Provider> {
        match self {
            Self::Override { provider, .. } => vec![*provider],
            Self::Default { chain } => chain.clone(),
        }
    }

    /// Первый кандидат — тот, кого берут без проверки готовности.
    ///
    /// Так поступают операции, у которых готовность исполнителя проверяет сам вызов
    /// (поиск платформы перед запуском); их квитанция пропущенных не содержит.
    pub fn first(&self) -> Option<Provider> {
        self.candidates().into_iter().next()
    }

    pub fn origin(&self) -> ProviderOrigin {
        match self {
            Self::Override { file, .. } => ProviderOrigin::Override { file: file.clone() },
            Self::Default { .. } => ProviderOrigin::Default,
        }
    }

    pub fn receipt_for(
        &self,
        selected: Provider,
        skipped: Vec<SkippedProvider>,
    ) -> ProviderReceipt {
        ProviderReceipt::new(selected, self.origin()).with_skipped(skipped)
    }

    /// Квитанция выбора, в котором никто не подошёл.
    pub fn receipt_for_nobody(&self, skipped: Vec<SkippedProvider>) -> ProviderReceipt {
        ProviderReceipt::nobody(self.origin(), skipped)
    }
}

/// Тестовая заготовка: `ibcmd` назначен всюду, где есть развилка на файловой базе.
///
/// Заменяет прежнее `builder: IBCMD` в конструкторах конфига внутри модульных тестов.
#[cfg(test)]
pub fn ibcmd_for_every_choice() -> std::collections::BTreeMap<Operation, Provider> {
    Operation::ALL
        .into_iter()
        .filter(|operation| {
            has_a_choice(*operation, TargetKind::File)
                && capability_of(*operation, TargetKind::File, Provider::Ibcmd).is_some()
        })
        .map(|operation| (operation, Provider::Ibcmd))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATRIX_ARTIFACT: &str = "docs/schemas/capability-matrix.json";

    /// Матрица как данные: операция → вид цели → исполнители строки по порядку с флагом
    /// `implemented`. Читает её сверка сайта `scripts/site_matrix.py`.
    fn matrix_document() -> serde_json::Value {
        let operations: serde_json::Map<String, serde_json::Value> = Operation::ALL
            .into_iter()
            .map(|operation| {
                let targets: serde_json::Map<String, serde_json::Value> = TargetKind::ALL
                    .into_iter()
                    .map(|target| {
                        let row = capabilities(operation, target)
                            .iter()
                            .map(|capability| {
                                serde_json::json!({
                                    "provider": capability.provider.as_str(),
                                    "implemented":
                                        capability.implementation == Implementation::Implemented,
                                })
                            })
                            .collect();
                        (target.as_str().to_owned(), serde_json::Value::Array(row))
                    })
                    .collect();
                (
                    operation.as_str().to_owned(),
                    serde_json::Value::Object(targets),
                )
            })
            .collect();
        serde_json::json!({
            "_comment": "Матрица исполнителей из src/domain/capability.rs. Порождается тестом при UPDATE_CAPABILITY_MATRIX=1, руками не правится.",
            "providers": Provider::ALL.map(Provider::as_str),
            "operations": operations,
        })
    }

    /// Артефакт матрицы совпадает с кодом. Обновление:
    /// `UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current`.
    #[test]
    fn generated_capability_matrix_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(MATRIX_ARTIFACT);
        let generated = matrix_document();
        if std::env::var_os("UPDATE_CAPABILITY_MATRIX").is_some() {
            let text = serde_json::to_string_pretty(&generated).expect("matrix serializes");
            std::fs::write(&path, format!("{text}\n")).expect("write capability matrix");
        }
        let text = std::fs::read_to_string(&path).expect("capability matrix artefact");
        let pinned: serde_json::Value =
            serde_json::from_str(&text).expect("capability matrix is valid json");
        assert_eq!(
            pinned, generated,
            "{MATRIX_ARTIFACT} is stale; rerun UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current"
        );
    }

    /// Умолчание — первый реализованный; экспериментальный в цепочку не входит.
    #[test]
    fn an_experimental_provider_never_leads_a_default_chain() {
        for operation in Operation::ALL {
            for target in [TargetKind::File, TargetKind::Cluster] {
                let chain = default_chain(operation, target);
                assert!(
                    !chain.is_empty(),
                    "{operation} on {} has no default provider",
                    target.as_str()
                );
                for provider in chain {
                    assert_eq!(
                        capability_of(operation, target, provider)
                            .map(|capability| capability.implementation),
                        Some(Implementation::Implemented)
                    );
                }
            }
        }
    }

    /// Строка не повторяет исполнителя: иначе «первый готовый» перестаёт быть однозначным.
    #[test]
    fn no_row_names_a_provider_twice() {
        for operation in Operation::ALL {
            for target in TargetKind::ALL {
                let row = capabilities(operation, target);
                let mut seen = std::collections::BTreeSet::new();
                for capability in row {
                    assert!(
                        seen.insert(capability.provider),
                        "{operation} on {} names {} twice",
                        target.as_str(),
                        capability.provider
                    );
                }
            }
        }
    }

    #[test]
    fn names_round_trip_through_their_strings() {
        for provider in Provider::ALL {
            assert_eq!(Provider::parse(provider.as_str()), Some(provider));
        }
        for operation in Operation::ALL {
            assert_eq!(Operation::parse(operation.as_str()), Some(operation));
        }
        assert_eq!(Provider::parse("designer-batch"), None);
    }

    /// Перечень видов цели полон: новый вид ломает сборку этого сопоставления, и автор
    /// видит рядом `TargetKind::ALL`, который надо дополнить.
    #[test]
    fn every_target_kind_is_listed() {
        for (position, target) in TargetKind::ALL.into_iter().enumerate() {
            let expected = match target {
                TargetKind::File => 0,
                TargetKind::Cluster => 1,
                TargetKind::Standalone => 2,
            };
            assert_eq!(position, expected, "{} out of place", target.as_str());
        }
    }

    #[test]
    fn a_receipt_keeps_the_override_file_and_the_skipped_reasons() {
        let plan = ProviderPlan::Override {
            provider: Provider::Ibcmd,
            file: "v8project.local.yaml".to_owned(),
        };
        let receipt = plan.receipt_for(Provider::Ibcmd, Vec::new());
        assert_eq!(
            receipt.origin,
            ProviderOrigin::Override {
                file: "v8project.local.yaml".to_owned()
            }
        );
        let plan = ProviderPlan::Default {
            chain: vec![Provider::Designer, Provider::Ibcmd],
        };
        let receipt = plan.receipt_for(
            Provider::Ibcmd,
            vec![SkippedProvider {
                provider: Provider::Designer,
                reason: "1cv8 not found".to_owned(),
            }],
        );
        assert_eq!(receipt.origin, ProviderOrigin::Default);
        assert_eq!(receipt.skipped.len(), 1);
    }
}
