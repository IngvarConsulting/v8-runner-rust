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
/// `convert`, `launch`, прогон тестов клиентом и `bootstrap` строк не имеют: у них один
/// инструмент, и назначать им исполнителя нечего.
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
    #[serde(rename = "publish")]
    Publish,
}

impl Operation {
    pub const ALL: [Self; 11] = [
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

    const DESIGNER_THEN_IBCMD: &[Capability] = &[
        implemented(Designer, LiveVerified),
        implemented(Ibcmd, ArgvTested),
    ];
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
    // `make`: у агента `dump-cfg` (cf/cfe) и сборка внешней обработки из файлов с
    // обратной выгрузкой — прогнаны раннером на 8.3.27 15–16.09.2026; `load`: у агента
    // нет `compare-cfg`, проба совместимости невозможна, строки нет намеренно.
    const MAKE: &[Capability] = &[
        implemented(Designer, LiveVerified),
        experimental(Agent, LiveVerified),
    ];
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
        (Operation::Init, TargetKind::File | TargetKind::Cluster) => DESIGNER_THEN_IBCMD,
        (Operation::Build, TargetKind::File | TargetKind::Cluster) => BUILD,
        (Operation::Dump, TargetKind::File | TargetKind::Cluster) => DUMP,
        (Operation::Load | Operation::Syntax, TargetKind::File | TargetKind::Cluster) => {
            DESIGNER_ONLY
        }
        (Operation::Make, TargetKind::File | TargetKind::Cluster) => MAKE,
        (Operation::Extensions, TargetKind::File | TargetKind::Cluster) => EXTENSIONS,
        (Operation::ConfigurationExport, TargetKind::File | TargetKind::Cluster) => EXPORT,
        (
            Operation::InfobaseDump | Operation::InfobaseRestore,
            TargetKind::File | TargetKind::Cluster,
        ) => SNAPSHOT,
        (Operation::Publish, TargetKind::File | TargetKind::Cluster) => WEBINST_ONLY,
        // Автономный сервер: единственная точка входа — его SSH-шлюз, тот же агентский
        // shell (`DEC.2026-09-14.ONLY-A-STANDALONE-SERVER-ANSWERS-WITHOUT-BEING-STARTED`).
        // Раннер к нему подключается, ничего не запуская, поэтому `init`, `publish`,
        // `load` (нет `compare-cfg`) и `syntax` строк не имеют. `infobase dump|restore`
        // строк не имеют намеренно: `infobase-tools dump-ib` через шлюз роняет `ibsrv`
        // 8.3.27 (SIGSEGV, живой прогон 15.09.2026), а `restore-ib` завершает сеанс
        // сервера по документации — снимок автономного сервера снимают его средствами.
        (
            Operation::Build
            | Operation::Dump
            | Operation::Make
            | Operation::Extensions
            | Operation::ConfigurationExport,
            TargetKind::Standalone,
        ) => GATE_ONLY,
        (_, TargetKind::Standalone) => &[],
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
}

impl ProviderReceipt {
    pub fn new(selected: Provider, origin: ProviderOrigin) -> Self {
        Self {
            selected: Some(selected),
            origin,
            skipped: Vec::new(),
        }
    }

    /// Никто не готов: пропущены все, кого пробовали.
    pub fn nobody(origin: ProviderOrigin, skipped: Vec<SkippedProvider>) -> Self {
        Self {
            selected: None,
            origin,
            skipped,
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
            for target in [
                TargetKind::File,
                TargetKind::Cluster,
                TargetKind::Standalone,
            ] {
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
