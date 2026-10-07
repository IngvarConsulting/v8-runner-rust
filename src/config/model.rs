use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::capability::{self, Operation, Provider, ProviderPlan, TargetKind};
use crate::domain::execution::ExecutionTimeouts;
use crate::platform::connection::V8Connection;
use crate::support::authority::{host_and_port_of_authority, Host};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// Root path of the project sources
    pub base_path: PathBuf,

    /// Working directory for temp files and hash storages
    pub work_path: PathBuf,

    /// Source format: DESIGNER or EDT
    #[serde(default = "default_format")]
    pub format: SourceFormat,

    /// Per-operation provider overrides: `providers.<operation>: <provider>`.
    ///
    /// The only way to name an executor by hand. A missing key means the default chain
    /// from the capability matrix; a present key means exactly that provider and no
    /// fallback.
    #[serde(default)]
    pub providers: BTreeMap<Operation, Provider>,

    /// Which file each override came from. Stamped by the loader, never read from YAML.
    #[serde(skip)]
    pub provider_origins: BTreeMap<Operation, String>,

    /// Declared infobases by name: the `infobases` map of the local overlay after the
    /// one-cycle `infobase:` synonym is folded into `origin`. Only the loader and a
    /// listing command read the map; every use case works with the selected `infobase`.
    #[serde(default)]
    pub infobases: BTreeMap<String, InfobaseConfig>,

    /// The infobase this run works with: the entry `infobases[infobase_name]`, or an ad
    /// hoc base built from `--infobase <connection string>`. The loader selects it before
    /// the config is deserialized, so a loaded config always has one.
    pub infobase: InfobaseConfig,

    /// Name of the selected infobase; `None` when it came as an ad hoc connection string.
    #[serde(default)]
    pub infobase_name: Option<String>,

    /// Source sets (configuration + extensions)
    #[serde(rename = "source-set", default)]
    pub source_sets: Vec<SourceSetConfig>,

    /// Platform tools configuration
    #[serde(default)]
    pub tools: ToolsConfig,

    /// MCP transport configuration
    #[serde(default)]
    pub mcp: McpConfig,

    /// Test pipeline configuration
    #[serde(default)]
    pub tests: TestsConfig,
}

/// Name of the infobase every command falls back to when `--infobase` is not given.
pub const DEFAULT_INFOBASE_NAME: &str = "origin";

/// Form of an infobase name: a plain identifier, because the name becomes a directory
/// under `workPath/infobases/` and a key of the `infobases` map. One source for the
/// validator and for the published schema (`INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER`).
pub const INFOBASE_NAME_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$";

/// Whether `name` has the form of an infobase name.
pub fn is_infobase_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= 64
        && first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What `--infobase` asked for: the default, a declared name, or a connection string.
///
/// A value in the form of a name is looked up in the `infobases` map; anything else is
/// taken as a connection string of an undeclared, ad hoc base.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InfobaseSelector {
    /// No flag: `origin`.
    #[default]
    Default,
    /// `--infobase <name>`: a declared infobase.
    Name(String),
    /// `--infobase <connection string>`: an ad hoc base without a name or credentials.
    Connection(String),
}

impl InfobaseSelector {
    /// Reads the `--infobase` flag; `None` means the default infobase.
    pub fn from_flag(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => Self::Default,
            Some(value) if is_infobase_name(value) => Self::Name(value.to_owned()),
            Some(value) => Self::Connection(value.to_owned()),
        }
    }
}

/// Connection and credentials for the target infobase.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InfobaseConfig {
    /// Connection string to the infobase: `File=…` or `Srvr=…;Ref=…`. Next to
    /// `standalone` it is the server's direct gate, by which the Designer reaches it, or
    /// empty.
    #[serde(default)]
    pub connection: String,

    /// Optional infobase user name passed to platform utilities.
    pub user: Option<String>,

    /// Optional infobase password passed to platform utilities.
    pub password: Option<String>,

    /// Optional DBMS contract for server-based infobases.
    #[serde(default)]
    pub dbms: Option<InfobaseDbmsConfig>,

    /// Client address and web-server publication settings.
    ///
    /// The runner administers the infobase through `connection`; a client or a browser
    /// opens it through `web.url`. For a file or cluster infobase the address appears
    /// after `publish`; a standalone server knows it up front.
    #[serde(default)]
    pub web: Option<InfobaseWebConfig>,

    /// Standalone server (`ibsrv`): the Designer reaches it by the direct gate in
    /// `connection`, the agent by its SSH gate. Its presence declares the target kind; the
    /// runner never starts the server.
    #[serde(default)]
    pub standalone: Option<StandaloneConfig>,

    /// The cluster around a server infobase: the administration server address and the
    /// two administrator levels above the infobase user
    /// (`DEC.2026-09-21.THE-CLUSTER-SECTION-HOLDS-RAS-AND-TWO-ADMIN-LEVELS`). Validation
    /// checks its form; no operation reads it yet (`sessions` — #212, the runner's own
    /// `ras` — #213, `infobase create` in a cluster — #204).
    #[serde(default)]
    pub cluster: Option<InfobaseClusterConfig>,

    /// Consent of this working copy to share a file infobase with the other copies that hold
    /// it (`INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY`). Only the local layer
    /// declares it: the loader refuses the key in the project file.
    #[serde(default)]
    pub shared: bool,
}

/// The cluster section of a server infobase. Every key is optional: each operation asks
/// only for the level it needs, and the refusal names the level that is missing.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InfobaseClusterConfig {
    /// Administration server (`ras`) address as `host[:port]`, the host a name or IPv4;
    /// it goes to `rac` as is, so the port default (1545) stays with the platform.
    #[serde(default)]
    pub ras: Option<String>,

    /// Cluster administrator name.
    #[serde(default)]
    pub user: Option<String>,

    /// Cluster administrator password.
    #[serde(default)]
    pub password: Option<String>,

    /// The central server agent and its administrator.
    #[serde(default)]
    pub agent: Option<InfobaseClusterAgentConfig>,
}

/// The central server agent (`ragent`) of the cluster and its administrator.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InfobaseClusterAgentConfig {
    /// Agent address as `host[:port]` when it differs from the host of `Srvr=` with the
    /// platform default port (1540).
    #[serde(default)]
    pub address: Option<String>,

    /// Central server administrator name.
    #[serde(default)]
    pub user: Option<String>,

    /// Central server administrator password.
    #[serde(default)]
    pub password: Option<String>,
}

/// A standalone server as the target. The Designer goes to its direct gate, declared by
/// `InfobaseConfig::connection`, and keeps its files on the runner's side; the agent
/// attaches to its SSH gate and exchanges files through a declared channel, never through
/// a path it assumes to be shared.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct StandaloneConfig {
    /// `host:port` or `[v6]:port` of the server's SSH gate (`ibsrv --enable-ssh-gate`); the
    /// port is required — `ibsrv` listens on 1543 unless told otherwise. Absent: the agent
    /// has no way to the server, and only the Designer serves it.
    #[serde(default)]
    pub gate: Option<String>,

    /// `SHA256:…` fingerprint the gate must present. Absent: the key is accepted and named.
    #[serde(default)]
    pub host_fingerprint: Option<String>,

    /// How files travel between the runner and the gate user's directory: `sftp` through
    /// the gate itself, or `{ dir: … }` — that directory as the runner sees it.
    #[serde(default)]
    pub exchange: Option<StandaloneExchangeConfig>,
}

/// The declared file channel to a standalone server.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum StandaloneExchangeConfig {
    /// A named channel: `sftp` — the SFTP subsystem of the gate's SSH connection.
    Named(StandaloneExchangeChannel),
    /// The gate user's directory (`<users-data>/<user>` of `ibsrv`) as the runner sees it:
    /// the server's own path on the same machine or a mount of it.
    Dir { dir: PathBuf },
}

/// Named exchange channels.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StandaloneExchangeChannel {
    Sftp,
}

impl StandaloneConfig {
    /// The gate as `(host, port)`; an undeclared gate is an error, for the caller asked
    /// for a way the declaration does not give.
    pub fn gate_endpoint(&self) -> Result<(Host, u16), String> {
        match self.gate.as_deref() {
            Some(gate) => ssh_endpoint(gate),
            None => Err("the SSH gate of the standalone server is not declared".to_owned()),
        }
    }

    /// The declared directory channel, if that is the channel.
    pub fn exchange_dir(&self) -> Option<&Path> {
        match self.exchange.as_ref() {
            Some(StandaloneExchangeConfig::Dir { dir }) => Some(dir.as_path()),
            _ => None,
        }
    }

    /// Files travel through the gate's SFTP subsystem.
    pub fn exchange_is_sftp(&self) -> bool {
        matches!(
            self.exchange,
            Some(StandaloneExchangeConfig::Named(
                StandaloneExchangeChannel::Sftp
            ))
        )
    }
}

/// The way a provider goes to a standalone server, and the key that declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandaloneWay {
    /// The Designer goes to the direct gate as to a cluster: `Srvr=<host>:<port>;Ref=<name>`
    /// in `infobase.connection`; its files stay on the runner's side.
    DirectGate,
    /// The agent attaches to the SSH gate `infobase.standalone.gate`; its files travel
    /// through the declared exchange channel.
    SshGate,
}

impl StandaloneWay {
    /// The way a provider of the standalone rows takes; `None` for a provider without one.
    pub const fn of(provider: Provider) -> Option<Self> {
        match provider {
            Provider::Designer => Some(Self::DirectGate),
            Provider::Agent => Some(Self::SshGate),
            Provider::Ibcmd | Provider::IbcmdRs | Provider::Webinst => None,
        }
    }

    /// The way as the refusal names it.
    const fn name(self) -> &'static str {
        match self {
            Self::DirectGate => "the direct gate",
            Self::SshGate => "the SSH gate",
        }
    }

    /// What the configuration declares for this way.
    const fn key(self) -> &'static str {
        match self {
            Self::DirectGate => "infobase.connection as Srvr=<host>:<port>;Ref=<name>",
            Self::SshGate => "infobase.standalone.gate",
        }
    }

    /// The one wording of a missing way: whom it lacks — a provider or a client — which way
    /// and what to declare. Every refusal and skip reason about an undeclared way is built
    /// from it.
    pub fn undeclared(self, who: impl std::fmt::Display) -> String {
        format!(
            "{who} reaches a standalone server by {}, which is not declared: declare {}",
            self.name(),
            self.key()
        )
    }
}

/// The endpoint of the runner's own SSH client: the gate of a standalone server or a
/// Designer agent started elsewhere. The `host:port` record is read by the one address
/// reader of the runner, `support::authority` (IPv6 in brackets, names lowercased, IPv4
/// canonical, whitespace and control characters refused); on top of it comes the
/// endpoint's own rule: the port is required, because the runner's client connects there
/// and no platform default applies.
fn ssh_endpoint(value: &str) -> Result<(Host, u16), String> {
    match host_and_port_of_authority(value) {
        Some((host, Some(port))) => Ok((host, port)),
        Some((_, None)) => Err(format!(
            "'{value}' has no port: the runner's own client connects there, so host:port or [v6]:port is required"
        )),
        None => Err(format!(
            "'{value}' must be a host with a port 1–65535 — host:port or [v6]:port"
        )),
    }
}

/// Port the central server agent (`ragent`) listens on unless told otherwise; the runner's
/// own `ras` (#213) is pointed at the host of `Srvr=` with this port.
pub(crate) const DEFAULT_CLUSTER_AGENT_PORT: u16 = 1540;

/// Where the cluster is administered from, as the runner derives it from the declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClusterAdministration {
    /// `cluster.ras` as declared: `rac` goes to this administration server.
    DeclaredRas(String),
    /// No `cluster.ras`: the runner starts its own `ras` against this agent (#213) —
    /// `cluster.agent.address`, or the first non-empty server of `Srvr=`; the port is
    /// [`DEFAULT_CLUSTER_AGENT_PORT`] unless `agent.address` names one.
    ManagedRasForAgent { host: Host, port: u16 },
}

/// Why no administration address follows from the declaration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ClusterAdministrationError {
    #[error(
        "infobase.connection names no cluster server, and infobase.cluster declares neither ras nor agent.address"
    )]
    NoServer,

    #[error(
        "infobase.connection names the cluster server '{server}', which is not `host` or `host:port`: declare infobase.cluster.ras or infobase.cluster.agent.address"
    )]
    ServerUnreadable { server: String },

    #[error(
        "infobase.cluster.agent.address '{address}' is not `host` or `host:port`, the host a name or IPv4"
    )]
    AgentAddressUnreadable { address: String },

    #[error(
        "infobase.connection names the cluster by the IPv6 address '{server}', and with infobase.cluster.ras and infobase.cluster.agent.address empty the runner takes the agent address from Srvr=: rac and ras accept only a name or IPv4 — declare infobase.cluster.ras or infobase.cluster.agent.address as a name or IPv4"
    )]
    ServerIsIpv6 { server: String },
}

/// The record `host[:port]` names an IPv6 host: it parses to an IPv6 address, or it is a
/// bare IPv6 address (which the authority reader does not take without brackets), or it
/// opens a bracket the reader refused. `srv::1545` and `tcp://srv:1545` are malformed
/// records, not IPv6.
pub(crate) fn names_an_ipv6_host(address: &str) -> bool {
    match host_and_port_of_authority(address) {
        Some((host, _)) => matches!(host, Host::Address(std::net::IpAddr::V6(_))),
        None => address.starts_with('[') || address.parse::<std::net::Ipv6Addr>().is_ok(),
    }
}

/// Web server a publication is written to.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebServerKind {
    Iis,
    Apache2,
    Apache22,
    Apache24,
}

impl WebServerKind {
    /// The `webinst` switch naming this server.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Iis => "iis",
            Self::Apache2 => "apache2",
            Self::Apache22 => "apache22",
            Self::Apache24 => "apache24",
        }
    }

    /// Apache 2.0 and 2.2 have no default configuration path `webinst` could guess.
    pub const fn requires_conf(self) -> bool {
        matches!(self, Self::Apache2 | Self::Apache22)
    }
}

/// Publication and client-address settings for the target infobase.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct InfobaseWebConfig {
    /// Web server to publish on.
    #[serde(default)]
    pub server: Option<WebServerKind>,

    /// Virtual directory name (`webinst -wsdir`).
    #[serde(default)]
    pub wsdir: Option<String>,

    /// Physical directory the publication is written to (`webinst -dir`).
    #[serde(default)]
    pub dir: Option<PathBuf>,

    /// Web server configuration file (`webinst -confpath`).
    #[serde(default)]
    pub conf: Option<PathBuf>,

    /// Use OS authentication (`webinst -osauth`, IIS only).
    #[serde(default, rename = "os-auth")]
    pub os_auth: bool,

    /// Address a client or a browser opens the infobase at.
    #[serde(default)]
    pub url: Option<String>,
}

impl InfobaseConfig {
    /// Адрес, к которому привязана память о базе, — без учётных данных и выбора исполнителя.
    /// У цели, которую приняла проверка конфигурации, он есть всегда
    /// (`INV.CONFIG.AN-ACCEPTED-TARGET-HAS-A-MEMORY-ADDRESS`); `None` — только у формы, которую
    /// проверка отвергает: такая цель память ни с кем не делит.
    pub fn memory_address(&self, base_path: &Path) -> Option<String> {
        // Автономный сервер помнится по SSH-шлюзу, если он объявлен: так память,
        // записанная до прямого шлюза, переживает появление строки рядом с секцией. Без
        // шлюза сервер помнится по строке прямого шлюза.
        let connection =
            || V8Connection::from_connection_string(&self.connection).snapshot_identity(base_path);
        match &self.standalone {
            Some(standalone) if standalone.gate.is_some() => standalone
                .gate_endpoint()
                .ok()
                .map(|(host, port)| format!("standalone:{host}:{port}")),
            Some(_) => connection().map(|identity| format!("standalone:{identity}")),
            None => connection(),
        }
    }

    /// Build a file-based infobase config.
    #[cfg(test)]
    pub fn file(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            user: None,
            password: None,
            dbms: None,
            web: None,
            standalone: None,
            cluster: None,
            shared: false,
        }
    }

    /// The administration address for `rac`, in the order the declaration answers it:
    /// `cluster.ras`; else `cluster.agent.address` for the runner's own `ras`; else the first
    /// non-empty server of `Srvr=` (or `/S`) with [`DEFAULT_CLUSTER_AGENT_PORT`]. Ask it only
    /// of a cluster target: a file base or a standalone server has no cluster to administer,
    /// and the answer for them is [`ClusterAdministrationError::NoServer`] or meaningless.
    /// The declared keys are
    /// already a name or IPv4 after validation
    /// (`INV.CONFIG.A-CLUSTER-ADDRESS-IS-A-NAME-OR-IPV4`); an IPv6 host taken from the
    /// connection string is refused here, because `rac` and `ras` do not work over IPv6.
    #[cfg_attr(not(test), allow(dead_code))] // called by `sessions` (#212) and the runner's `ras` (#213)
    pub(crate) fn cluster_administration(
        &self,
    ) -> Result<ClusterAdministration, ClusterAdministrationError> {
        let cluster = self.cluster.as_ref();
        if let Some(ras) = cluster.and_then(|cluster| cluster.ras.as_deref()) {
            return Ok(ClusterAdministration::DeclaredRas(ras.to_owned()));
        }
        if let Some(agent) = cluster
            .and_then(|cluster| cluster.agent.as_ref())
            .and_then(|agent| agent.address.as_deref())
        {
            return match host_and_port_of_authority(agent) {
                Some((host, port)) if !names_an_ipv6_host(agent) => {
                    Ok(ClusterAdministration::ManagedRasForAgent {
                        host,
                        port: port.unwrap_or(DEFAULT_CLUSTER_AGENT_PORT),
                    })
                }
                _ => Err(ClusterAdministrationError::AgentAddressUnreadable {
                    address: agent.to_owned(),
                }),
            };
        }
        let hosts = V8Connection::from_connection_string(&self.connection).cluster_hosts();
        let server = hosts
            .into_iter()
            .find(|server| !server.is_empty())
            .ok_or(ClusterAdministrationError::NoServer)?;
        if names_an_ipv6_host(&server) {
            return Err(ClusterAdministrationError::ServerIsIpv6 { server });
        }
        match host_and_port_of_authority(&server) {
            Some((host, _)) => Ok(ClusterAdministration::ManagedRasForAgent {
                host,
                port: DEFAULT_CLUSTER_AGENT_PORT,
            }),
            None => Err(ClusterAdministrationError::ServerUnreadable { server }),
        }
    }

    /// Attach infobase credentials to an existing config.
    #[cfg(test)]
    pub fn with_credentials(mut self, user: Option<String>, password: Option<String>) -> Self {
        self.user = user;
        self.password = password;
        self
    }

    /// Build a server-based infobase config.
    #[cfg(test)]
    pub fn server(connection: impl Into<String>, dbms: InfobaseDbmsConfig) -> Self {
        Self {
            connection: connection.into(),
            user: None,
            password: None,
            web: None,
            standalone: None,
            dbms: Some(dbms),
            cluster: None,
            shared: false,
        }
    }
}

/// DBMS-level contract used by `IBCMD` for server-based infobases.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct InfobaseDbmsConfig {
    /// DBMS kind passed as `--dbms`.
    #[serde(default)]
    pub kind: Option<String>,

    /// DBMS server passed as `--database-server`.
    #[serde(default)]
    pub server: Option<String>,

    /// Physical database name passed as `--database-name`.
    #[serde(default)]
    pub name: Option<String>,

    /// Optional DBMS user passed as `--database-user`.
    #[serde(default)]
    pub user: Option<String>,

    /// Optional DBMS password passed as `--database-password`.
    #[serde(default)]
    pub password: Option<String>,

    /// National settings of a new infobase in a cluster: `Locale=` of `CREATEINFOBASE`.
    #[serde(default)]
    pub locale: Option<String>,
}

/// Обязательное поле секции `infobase.dbms`, которого нет: раннер идёт в СУБД сам и берёт
/// его из секции. Текст один у всех, кто читает контракт.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("infobase.dbms.{} is not declared: the runner goes to the DBMS itself and takes {} from the dbms section{}", .field.key(), .field.meaning(), .field.consequence())]
pub struct MissingDbmsField {
    pub field: DbmsField,
}

/// Обязательное поле секции `infobase.dbms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbmsField {
    Kind,
    Server,
    Name,
    /// Нужно только созданию базы в кластере.
    Locale,
}

impl DbmsField {
    /// Ключ поля в секции.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Server => "server",
            Self::Name => "name",
            Self::Locale => "locale",
        }
    }

    const fn meaning(self) -> &'static str {
        match self {
            Self::Kind => "the DBMS kind",
            Self::Server => "the DBMS server",
            Self::Name => "the database name",
            Self::Locale => "the locale of a new cluster infobase",
        }
    }

    const fn consequence(self) -> &'static str {
        match self {
            Self::Kind | Self::Server | Self::Name => "",
            Self::Locale => {
                " — without Locale CREATEINFOBASE leaves an abandoned database in the DBMS"
            }
        }
    }
}

/// Непустое имя или пароль: имя из одних пробелов — не имя. Одно правило для учётных
/// записей СУБД и кластера.
pub(crate) fn declared_name(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

/// Проверенный доступ к СУБД — единственное чтение контракта `infobase.dbms`: обязательные
/// поля непусты и без пробелов по краям, необязательные пустыми не передаются.
#[derive(Clone, Copy)]
pub struct DbmsAccess<'a> {
    pub kind: &'a str,
    pub server: &'a str,
    pub name: &'a str,
    pub user: Option<&'a str>,
    pub password: Option<&'a str>,
}

impl InfobaseConfig {
    /// Доступ к СУБД из секции `dbms`; без секции не хватает первого поля — `kind`.
    pub fn dbms_access(&self) -> Result<DbmsAccess<'_>, MissingDbmsField> {
        let dbms = self.dbms.as_ref().ok_or(MissingDbmsField {
            field: DbmsField::Kind,
        })?;
        Ok(DbmsAccess {
            kind: required_dbms_field(DbmsField::Kind, dbms.kind.as_deref())?,
            server: required_dbms_field(DbmsField::Server, dbms.server.as_deref())?,
            name: required_dbms_field(DbmsField::Name, dbms.name.as_deref())?,
            user: declared_name(dbms.user.as_deref()),
            password: dbms
                .password
                .as_deref()
                .filter(|password| !password.is_empty()),
        })
    }

    /// Национальные настройки новой базы в кластере (`dbms.locale`).
    pub fn dbms_locale(&self) -> Result<&str, MissingDbmsField> {
        required_dbms_field(
            DbmsField::Locale,
            self.dbms.as_ref().and_then(|dbms| dbms.locale.as_deref()),
        )
    }
}

fn required_dbms_field(field: DbmsField, value: Option<&str>) -> Result<&str, MissingDbmsField> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(MissingDbmsField { field })
}

impl InfobaseDbmsConfig {
    /// Build a DBMS contract with mandatory fields populated.
    #[cfg(test)]
    pub fn new(
        kind: impl Into<String>,
        server: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: Some(kind.into()),
            server: Some(server.into()),
            name: Some(name.into()),
            user: None,
            password: None,
            locale: None,
        }
    }

    /// Attach DBMS credentials to an existing contract.
    #[cfg(test)]
    pub fn with_credentials(mut self, user: Option<String>, password: Option<String>) -> Self {
        self.user = user;
        self.password = password;
        self
    }
}

impl AppConfig {
    /// Builds a platform-ready 1C connection with infobase credentials applied.
    pub fn v8_connection(&self) -> V8Connection {
        let mut conn = V8Connection::from_connection_string(&self.infobase.connection);
        conn.user = self.infobase.user.clone();
        conn.password = self.infobase.password.clone();
        conn
    }

    /// Address that hash memory is bound to, without credentials or executor choice.
    pub fn infobase_memory_address(&self, base_path: &Path) -> Option<String> {
        self.infobase.memory_address(base_path)
    }

    /// `infobase.connection` is declared. Only a standalone server may leave it empty: its
    /// connection string is the direct gate, and the section may declare the SSH gate alone.
    pub fn connection_declared(&self) -> bool {
        !self.infobase.connection.trim().is_empty()
    }

    /// Kind of the target infobase, as declared by the connection contract.
    pub fn target_kind(&self) -> TargetKind {
        if self.infobase.standalone.is_some() {
            TargetKind::Standalone
        } else if self.v8_connection().file_path().is_some() {
            TargetKind::File
        } else {
            TargetKind::Cluster
        }
    }

    /// The way to the target this provider lacks for the operation, as declared. Only a
    /// standalone server can lack one: the Designer goes to it by the direct gate in
    /// `connection`, the agent by `standalone.gate`, and either may be left undeclared.
    /// `None` — the way is declared, the operation does not go to the target (`make`
    /// builds in a throwaway base of the runner), or the target is not a standalone server.
    pub fn missing_way(&self, operation: Operation, provider: Provider) -> Option<StandaloneWay> {
        if capability::needs_no_target(operation) {
            return None;
        }
        let standalone = self.infobase.standalone.as_ref()?;
        let way = StandaloneWay::of(provider)?;
        let declared = match way {
            StandaloneWay::DirectGate => self.connection_declared(),
            StandaloneWay::SshGate => standalone.gate.is_some(),
        };
        (!declared).then_some(way)
    }

    /// Why the matrix row of an operation is left empty on this standalone server: every
    /// provider of the row lacks its way, and the refusal names what to declare. `None` —
    /// the row has a provider with a declared way, the row is empty in the matrix itself,
    /// or the target is not a standalone server.
    pub fn undeclared_way(&self, operation: Operation) -> Option<String> {
        let row = capability::default_chain(operation, self.target_kind());
        if row.is_empty()
            || row
                .iter()
                .any(|provider| self.missing_way(operation, *provider).is_none())
        {
            return None;
        }
        let ways = row
            .iter()
            .filter_map(|provider| {
                self.missing_way(operation, *provider)
                    .map(|way| way.undeclared(*provider))
            })
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!(
            "{operation} has no executor with a declared way to the standalone server: {ways}"
        ))
    }

    /// Who is assigned to an operation on this target, before any readiness check.
    ///
    /// An override names one provider and never falls back; a default is the matrix
    /// chain, from which the caller takes the first ready one. On a standalone server the
    /// chain keeps only the providers whose way to it is declared.
    pub fn provider_plan(&self, operation: Operation) -> ProviderPlan {
        match self.providers.get(&operation) {
            Some(provider) => ProviderPlan::Override {
                provider: *provider,
                file: self
                    .provider_origins
                    .get(&operation)
                    .cloned()
                    .unwrap_or_else(|| crate::config::loader::DEFAULT_CONFIG_FILE_NAME.to_owned()),
            },
            None => ProviderPlan::Default {
                chain: self.default_chain_shaped(operation, self.project_shape()),
            },
        }
    }

    /// Форма проекта, которая сужает цепочки умолчаний (`capability::serves_project`).
    pub fn project_shape(&self) -> capability::ProjectShape {
        capability::ProjectShape {
            edt_sources: self.format == SourceFormat::Edt,
            tool_extension: self.tools.client_mcp.extension.is_some(),
        }
    }

    /// Исполнитель `push` при объявленном расширении-инструменте: так спрашивает тот, кто
    /// расширение только собирается объявить (`tools download client-mcp`).
    pub fn push_provider_with_tool_extension(&self) -> Option<Provider> {
        match self.providers.get(&Operation::Build) {
            Some(provider) => Some(*provider),
            None => self
                .default_chain_shaped(
                    Operation::Build,
                    capability::ProjectShape {
                        tool_extension: true,
                        ..self.project_shape()
                    },
                )
                .into_iter()
                .next(),
        }
    }

    /// Цепочка умолчаний для проекта данной формы: строка матрицы без исполнителей, которые
    /// такой проект не обслуживают, и без тех, чей путь к автономному серверу не объявлен.
    fn default_chain_shaped(
        &self,
        operation: Operation,
        shape: capability::ProjectShape,
    ) -> Vec<Provider> {
        capability::default_chain_for(operation, self.target_kind(), shape)
            .into_iter()
            .filter(|provider| self.missing_way(operation, *provider).is_none())
            .collect()
    }

    /// The provider an operation would dispatch to, or `None` where the target has no
    /// executor for it at all (a standalone server has no `load`, `init`, `syntax`).
    pub fn default_provider(&self, operation: Operation) -> Option<Provider> {
        self.provider_plan(operation).first()
    }

    /// The provider an operation dispatches to when it does not probe readiness itself.
    ///
    /// Validation guarantees the matrix has a row for every operation on file and cluster
    /// targets, so an empty chain here is a programming error, not a user one. Where the
    /// target may lack a row, ask `default_provider` instead.
    pub fn selected_provider(&self, operation: Operation) -> Provider {
        self.provider_plan(operation).first().unwrap_or_else(|| {
            panic!(
                "no provider row for {operation} on {}",
                self.target_kind().as_str()
            )
        })
    }

    /// Returns the client MCP wait-ready timeout as a duration.
    pub fn client_mcp_wait_ready_timeout_duration(&self) -> Duration {
        Duration::from_millis(
            self.tools
                .client_mcp
                .wait_ready_timeout_ms
                .unwrap_or_else(default_client_mcp_wait_ready_timeout_ms)
                .max(1),
        )
    }

    /// Returns how long an MCP call may wait for a free execution slot.
    pub fn mcp_admission_timeout_duration(&self) -> Duration {
        Duration::from_millis(self.mcp.execution.admission_timeout_ms.max(1))
    }
}

fn default_format() -> SourceFormat {
    SourceFormat::Designer
}

fn default_client_mcp_wait_ready_timeout_ms() -> u64 {
    300_000
}

fn default_mcp_execution_admission_timeout_ms() -> u64 {
    300_000
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceFormat {
    Designer,
    Edt,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceSetConfig {
    pub name: String,

    /// YAML `type`: CONFIGURATION, EXTENSION, EXTERNAL_DATA_PROCESSORS, or EXTERNAL_REPORTS.
    #[serde(rename = "type")]
    pub purpose: SourceSetPurpose,

    /// Path relative to the project base path (for DESIGNER) or EDT project path.
    pub path: PathBuf,
}

impl SourceSetConfig {
    /// Каталог source-set: `path` считается от `base_path` тем же правилом, что и остальные
    /// пути конфига ([`crate::support::path::resolve_from`]).
    pub fn root_in(&self, base_path: &Path) -> PathBuf {
        crate::support::path::resolve_from(base_path, &self.path)
    }
}

/// Назначение набора живёт в домене: его называет и ответ `status`.
pub use crate::domain::source_set::SourceSetPurpose;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub platform: PlatformToolConfig,

    #[serde(default)]
    pub enterprise: EnterpriseToolConfig,

    #[serde(rename = "edt_cli", default)]
    pub edt_cli: EdtCliConfig,

    /// Designer agent endpoint: launched by the runner or attached to.
    #[serde(rename = "designer_agent", default)]
    pub designer_agent: DesignerAgentConfig,

    #[serde(default)]
    pub client_mcp: ClientMcpToolConfig,

    #[serde(default)]
    pub va: VanessaToolConfig,
}

/// MCP transport-neutral runtime configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
#[derive(Default)]
pub struct McpConfig {
    /// HTTP transport settings for the future MCP server.
    pub http: McpHttpConfig,

    /// Shared execution limits for MCP calls.
    pub execution: McpExecutionConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct ClientMcpToolConfig {
    /// Default port passed to onec-client-mcp-devkit via `/C ...;mcpPort=<PORT>`.
    pub port: Option<u16>,

    /// Optional wait-ready timeout in milliseconds. Defaults to five minutes when unset.
    pub wait_ready_timeout_ms: Option<u64>,

    /// Optional tool extension prepared by `push` for client MCP launches.
    pub extension: Option<ToolExtensionConfig>,
}

#[derive(Debug, Clone)]
pub struct ToolExtensionConfig {
    /// Extension name in the target infobase.
    pub name: String,

    /// Mutually exclusive extension input.
    pub input: ToolExtensionInput,
}

impl ToolExtensionConfig {
    pub fn source(&self) -> Option<&ToolExtensionSourceConfig> {
        match &self.input {
            ToolExtensionInput::Source(source) => Some(source),
            ToolExtensionInput::Artifact(_) => None,
        }
    }

    pub fn source_mut(&mut self) -> Option<&mut ToolExtensionSourceConfig> {
        match &mut self.input {
            ToolExtensionInput::Source(source) => Some(source),
            ToolExtensionInput::Artifact(_) => None,
        }
    }

    pub fn artifact_mut(&mut self) -> Option<&mut ToolExtensionArtifactConfig> {
        match &mut self.input {
            ToolExtensionInput::Source(_) => None,
            ToolExtensionInput::Artifact(artifact) => Some(artifact),
        }
    }
}

impl<'de> Deserialize<'de> for ToolExtensionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "snake_case")]
        struct RawToolExtensionConfig {
            name: String,
            source: Option<ToolExtensionSourceConfig>,
            artifact: Option<ToolExtensionArtifactConfig>,
        }

        let raw = RawToolExtensionConfig::deserialize(deserializer)?;
        let input = match (raw.source, raw.artifact) {
            (Some(source), None) => ToolExtensionInput::Source(source),
            (None, Some(artifact)) => ToolExtensionInput::Artifact(artifact),
            (Some(_), Some(_)) => {
                return Err(D::Error::custom(
                    "tools.client_mcp.extension must specify exactly one of source or artifact",
                ))
            }
            (None, None) => {
                return Err(D::Error::custom(
                    "tools.client_mcp.extension must specify exactly one of source or artifact",
                ))
            }
        };

        Ok(Self {
            name: raw.name,
            input,
        })
    }
}

impl Serialize for ToolExtensionConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ToolExtensionConfig", 2)?;
        state.serialize_field("name", &self.name)?;
        match &self.input {
            ToolExtensionInput::Source(source) => state.serialize_field("source", source)?,
            ToolExtensionInput::Artifact(artifact) => {
                state.serialize_field("artifact", artifact)?
            }
        }
        state.end()
    }
}

#[derive(Debug, Clone)]
pub enum ToolExtensionInput {
    Source(ToolExtensionSourceConfig),
    Artifact(ToolExtensionArtifactConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ToolExtensionSourceConfig {
    /// Path to extension sources.
    pub path: PathBuf,

    /// Optional source format. When omitted, the project-level `format` is used.
    pub format: Option<SourceFormat>,
}

impl Default for ToolExtensionSourceConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            format: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ToolExtensionArtifactConfig {
    /// Path to a `.cfe` artifact.
    pub path: PathBuf,
}

impl Default for ToolExtensionArtifactConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct VanessaToolConfig {
    /// Path to the Vanessa Automation external data processor used by `test va` and `launch mcp va`.
    pub epf_path: Option<PathBuf>,
}

/// HTTP-specific MCP configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct McpHttpConfig {
    /// Socket address for the future HTTP transport listener.
    pub bind_address: String,

    /// URL path that serves MCP HTTP requests.
    pub path: String,

    /// Whether MCP HTTP sessions keep state across requests.
    pub stateful_sessions: bool,

    /// Maximum number of tracked HTTP sessions.
    pub max_sessions: usize,

    /// Idle session eviction timeout in seconds.
    pub idle_ttl_secs: u64,

    /// Hosts the HTTP listener answers besides the loopback, as `Host` header values.
    pub allowed_hosts: Vec<String>,
}

impl Default for McpHttpConfig {
    fn default() -> Self {
        Self {
            bind_address: default_mcp_http_bind_address(),
            path: default_mcp_http_path(),
            stateful_sessions: default_mcp_http_stateful_sessions(),
            max_sessions: default_mcp_http_max_sessions(),
            idle_ttl_secs: default_mcp_http_idle_ttl_secs(),
            allowed_hosts: Vec::new(),
        }
    }
}

/// Execution guardrails for MCP requests.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct McpExecutionConfig {
    /// Maximum number of MCP calls allowed to execute concurrently.
    pub max_concurrent_calls: usize,

    /// Grace period for shutdown drain in seconds.
    pub shutdown_grace_period_secs: u64,

    /// How long an MCP call may wait for a free execution slot, in milliseconds.
    ///
    /// Bounds admission only. Work that has been admitted runs to its terminal outcome:
    /// see DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE.
    #[serde(default = "default_mcp_execution_admission_timeout_ms")]
    pub admission_timeout_ms: u64,
}

impl Default for McpExecutionConfig {
    fn default() -> Self {
        Self {
            max_concurrent_calls: default_mcp_execution_max_concurrent_calls(),
            shutdown_grace_period_secs: default_mcp_execution_shutdown_grace_period_secs(),
            admission_timeout_ms: default_mcp_execution_admission_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "snake_case")]
pub struct TestsConfig {
    #[serde(default = "default_test_execution_timeout_seconds")]
    pub execution_timeout_seconds: u64,

    #[serde(default)]
    pub yaxunit: YaxunitTestConfig,

    #[serde(default)]
    pub va: VanessaTestConfig,
}

impl Default for TestsConfig {
    fn default() -> Self {
        Self {
            execution_timeout_seconds: default_test_execution_timeout_seconds(),
            yaxunit: YaxunitTestConfig::default(),
            va: VanessaTestConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct YaxunitTestConfig {
    pub timeouts: ExecutionTimeouts,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct VanessaTestConfig {
    pub params_path: Option<PathBuf>,
    pub profile: Option<String>,
    pub fail_fast: bool,
    pub timeouts: ExecutionTimeouts,
    pub profiles: BTreeMap<String, VanessaProfileConfig>,
}

impl VanessaTestConfig {
    pub fn is_configured(&self) -> bool {
        self.params_path.is_some() || self.profile.is_some() || !self.profiles.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, rename_all = "snake_case")]
pub struct VanessaProfileConfig {
    pub feature_path: Option<PathBuf>,
    pub features_to_run: Vec<String>,
    pub filter_tags: Vec<String>,
    pub ignore_tags: Vec<String>,
    pub scenario_filter: Vec<String>,
}

fn default_test_execution_timeout_seconds() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PlatformToolConfig {
    /// Installation hint for platform utilities.
    ///
    /// May point to a concrete binary (`1cv8`, `1cv8c`, `ibcmd`), to an installation `bin`
    /// directory, or to a platform root that contains versioned subdirectories.
    pub path: Option<PathBuf>,

    /// Enforce `version` for a configured `path`.
    ///
    /// `path` is always an explicit-only search boundary. When `strict` is `false`,
    /// `version` is ignored for that path; when `strict` is `true`, the executable
    /// found inside the path must match `version`. Without `path`, `version` is
    /// applied to normal default-root and PATH discovery.
    #[serde(default)]
    pub strict: bool,

    /// Platform version requirement in `major.minor`, `major.minor.patch`, or
    /// `major.minor.patch.build` format.
    ///
    /// A 2- or 3-part value selects the highest matching version; a 4-part value
    /// selects an exact build.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct EnterpriseToolConfig {
    /// Additional command-line keys appended to enterprise client launches.
    #[serde(default)]
    pub additional_launch_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct EdtCliConfig {
    /// Path to 1cedtcli binary, installation root, or version-like discovery hint.
    pub path: Option<PathBuf>,

    /// Optional EDT version hint used for auto-discovery, for example `1c-edt-2025.2.3`.
    pub version: Option<String>,

    /// Use long-lived interactive `1cedtcli` processes instead of one-shot invocations.
    #[serde(default)]
    pub interactive_mode: bool,

    /// Eagerly prewarm the shared EDT session on MCP server startup.
    ///
    /// Short-lived CLI commands ignore this flag and start EDT lazily on demand.
    #[serde(default)]
    pub auto_start: bool,

    /// Time limit for EDT startup until the prompt is ready.
    #[serde(
        default = "default_edt_cli_startup_timeout_ms",
        rename = "startup_timeout_ms"
    )]
    pub startup_timeout_ms: u64,

    /// Default timeout for interactive EDT commands.
    #[serde(
        default = "default_edt_cli_command_timeout_ms",
        rename = "command_timeout_ms"
    )]
    pub command_timeout_ms: u64,
}

impl EdtCliConfig {
    /// `edt_cli.path` — путь, а не голое имя без каталога. Голое имя (`1cedtcli`,
    /// `1c-edt-2025.2.3`) остаётся подсказкой автопоиска EDT; всё остальное загрузчик
    /// считает от каталога конфига. Одно правило для загрузчика и поиска утилит.
    pub fn names_location(path: &Path) -> bool {
        path.is_absolute() || path.components().count() > 1
    }
}

impl Default for EdtCliConfig {
    fn default() -> Self {
        Self {
            path: None,
            version: None,
            interactive_mode: false,
            auto_start: false,
            startup_timeout_ms: default_edt_cli_startup_timeout_ms(),
            command_timeout_ms: default_edt_cli_command_timeout_ms(),
        }
    }
}

/// Where the Designer agent lives and how the runner reaches it.
///
/// Two modes, told apart by the keys present: `attach` names an agent somebody else
/// started, everything else describes the agent the runner launches itself. The two
/// sets of keys do not mix; the loader refuses a config that names both.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct DesignerAgentConfig {
    /// `host:port` or `[v6]:port` of an agent started outside the runner; the port is
    /// required. Attached mode.
    pub attach: Option<String>,

    /// `AgentBaseDir` of the attached agent, where its commands read and write files.
    /// Attached mode only; the managed agent always works under `workPath`.
    pub base_dir: Option<PathBuf>,

    /// Port the managed agent listens on. Managed mode; absent: a free loopback port chosen
    /// for each launch.
    pub port: Option<u16>,

    /// Private host key for the managed agent. Absent: a one-time ED25519 key generated for
    /// each launch under `workPath`, handed to the agent and pinned for the session.
    pub host_key: Option<PathBuf>,

    /// `SHA256:…` fingerprint the attached agent must present. Attached mode only:
    /// the managed agent's key is the one the runner hands it in `host-key`.
    pub host_fingerprint: Option<String>,

    /// Time limit for the managed agent to accept the first authenticated session.
    #[serde(
        default = "default_designer_agent_startup_timeout_ms",
        rename = "startup_timeout_ms"
    )]
    pub startup_timeout_ms: u64,
}

impl Default for DesignerAgentConfig {
    fn default() -> Self {
        Self {
            attach: None,
            base_dir: None,
            port: None,
            host_key: None,
            host_fingerprint: None,
            startup_timeout_ms: default_designer_agent_startup_timeout_ms(),
        }
    }
}

/// The mode the keys of `tools.designer_agent` describe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignerAgentMode {
    /// The runner launches `1cv8 DESIGNER … /AgentMode` and owns its lifetime.
    /// `port: None` — свободный порт на каждый запуск.
    Managed { port: Option<u16> },
    /// The runner connects to an agent it did not start and never restarts it.
    Attached { host: Host, port: u16 },
}

impl DesignerAgentConfig {
    /// Keys that only make sense for a managed agent.
    pub fn managed_keys_present(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.port.is_some() {
            keys.push("port");
        }
        if self.host_key.is_some() {
            keys.push("host-key");
        }
        keys
    }

    /// Keys that only make sense for an attached agent.
    pub fn attached_keys_present(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.base_dir.is_some() {
            keys.push("base-dir");
        }
        if self.host_fingerprint.is_some() {
            keys.push("host-fingerprint");
        }
        keys
    }

    /// Mode derived from the keys; `attach` that does not parse is reported as such.
    pub fn mode(&self) -> Result<DesignerAgentMode, String> {
        match self.attach.as_deref() {
            None => Ok(DesignerAgentMode::Managed { port: self.port }),
            Some(attach) => {
                let (host, port) = ssh_endpoint(attach)?;
                Ok(DesignerAgentMode::Attached { host, port })
            }
        }
    }
}

const fn default_designer_agent_startup_timeout_ms() -> u64 {
    120_000
}

fn default_mcp_http_bind_address() -> String {
    "127.0.0.1:3000".to_owned()
}

fn default_mcp_http_path() -> String {
    "/mcp".to_owned()
}

const fn default_mcp_http_stateful_sessions() -> bool {
    true
}

const fn default_mcp_http_max_sessions() -> usize {
    64
}

const fn default_mcp_http_idle_ttl_secs() -> u64 {
    900
}

const fn default_mcp_execution_max_concurrent_calls() -> usize {
    1
}

const fn default_mcp_execution_shutdown_grace_period_secs() -> u64 {
    30
}

const fn default_edt_cli_startup_timeout_ms() -> u64 {
    300_000
}

const fn default_edt_cli_command_timeout_ms() -> u64 {
    300_000
}

#[cfg(test)]
mod tests {
    use super::{
        names_an_ipv6_host, ssh_endpoint, ClusterAdministration, ClusterAdministrationError,
        DesignerAgentConfig, DesignerAgentMode, EdtCliConfig, Host, InfobaseConfig,
        PlatformToolConfig, StandaloneConfig,
    };

    #[test]
    fn edt_cli_path_is_a_location_unless_it_is_a_bare_name() {
        use std::path::Path;
        for bare in ["1cedtcli", "1c-edt-2025.2.3", "2025.2.3"] {
            assert!(!EdtCliConfig::names_location(Path::new(bare)), "{bare}");
        }
        for location in ["./1cedtcli", "tools/edt", "/opt/edt/1cedtcli"] {
            assert!(
                EdtCliConfig::names_location(Path::new(location)),
                "{location}"
            );
        }
    }

    #[test]
    fn platform_strict_defaults_to_false_and_deserializes_true() {
        let default = PlatformToolConfig::default();
        assert!(!default.strict);

        let configured: PlatformToolConfig =
            serde_yaml::from_str("strict: true\n").expect("deserialize strict platform config");
        assert!(configured.strict);
    }

    /// Шлюз и `attach` читаются одним читателем адреса: скобки IPv6 снимаются, имя —
    /// строчными и в punycode, IPv4 — канонический; порт обязателен; пробелы — не адрес.
    #[test]
    fn the_gate_and_the_attach_endpoint_are_read_by_the_authority_reader() {
        let address = |value: &str| Host::Address(value.parse().expect("literal address"));
        let name = |value: &str| Host::Name(value.to_owned());
        for (record, host, port) in [
            ("[::1]:1543", address("::1"), 1543),
            ("127.0.0.1:1543", address("127.0.0.1"), 1543),
            ("0177.0.0.1:1543", address("127.0.0.1"), 1543),
            ("SRV.example.:1543", name("srv.example"), 1543),
            ("сервер:1543", name("xn--b1afb6bcb"), 1543),
        ] {
            let standalone: StandaloneConfig =
                serde_yaml::from_str(&format!("gate: '{record}'\n")).expect("standalone");
            assert_eq!(
                standalone.gate_endpoint(),
                Ok((host.clone(), port)),
                "{record}"
            );
            let agent: DesignerAgentConfig =
                serde_yaml::from_str(&format!("attach: '{record}'\n")).expect("agent");
            assert_eq!(
                agent.mode(),
                Ok(DesignerAgentMode::Attached { host, port }),
                "{record}"
            );
        }
        for (record, reason) in [
            ("srv", "has no port"),
            ("[::1]", "has no port"),
            ("srv:", "must be a host with a port"),
            ("srv:0", "must be a host with a port"),
            ("srv:x", "must be a host with a port"),
            ("::1:1543", "must be a host with a port"),
            ("", "must be a host with a port"),
            (" srv:1543", "must be a host with a port"),
            ("srv:1543 ", "must be a host with a port"),
        ] {
            let error = ssh_endpoint(record).expect_err(record);
            assert!(
                error.contains(reason) && error.contains(&format!("'{record}'")),
                "{record}: {error}"
            );
        }
    }

    fn agent_at(host: &str, port: u16) -> ClusterAdministration {
        ClusterAdministration::ManagedRasForAgent {
            host: crate::support::authority::host_of_authority(host).expect("agent host"),
            port,
        }
    }

    fn cluster_base(connection: &str, cluster: &str) -> InfobaseConfig {
        let mut base = InfobaseConfig::file(connection);
        base.cluster = Some(serde_yaml::from_str(cluster).expect("cluster section"));
        base
    }

    /// Адрес администрирования выводится по порядку: `cluster.ras`, иначе
    /// `cluster.agent.address`, иначе первый непустой сервер `Srvr=` с портом агента; IPv6 из строки
    /// подключения — отказ, а объявленный ключ строку не читает вовсе (#213).
    #[test]
    fn the_cluster_administration_address_is_derived_in_the_declared_order() {
        let ipv6_server = "Srvr=[::1]:1541;Ref=demo";
        assert_eq!(
            cluster_base(
                ipv6_server,
                "{ras: 'ras-host:1545', agent: {address: agent-host}}"
            )
            .cluster_administration(),
            Ok(ClusterAdministration::DeclaredRas(
                "ras-host:1545".to_owned()
            )),
            "ras wins"
        );
        assert_eq!(
            cluster_base(
                "Srvr=srv:1541;Ref=demo",
                "{agent: {address: 'agent-host:2540'}}"
            )
            .cluster_administration(),
            Ok(agent_at("agent-host", 2540)),
            "agent.address wins over Srvr="
        );
        assert_eq!(
            cluster_base(ipv6_server, "{agent: {address: agent-host}}").cluster_administration(),
            Ok(agent_at("agent-host", super::DEFAULT_CLUSTER_AGENT_PORT)),
            "a declared agent name is not refused for an IPv6 Srvr="
        );

        for (connection, agent) in [
            ("Srvr=SRV:1541;Ref=demo", "srv"),
            ("Srvr=10.0.0.5;Ref=demo", "10.0.0.5"),
            ("Srvr='tcp://srv1:1541,[::1]:1541';Ref=demo", "srv1"),
            ("Srvr=' ,srv2';Ref=demo", "srv2"),
            ("/S srv:1541\\demo", "srv"),
        ] {
            assert_eq!(
                cluster_base(connection, "{}").cluster_administration(),
                Ok(agent_at(agent, super::DEFAULT_CLUSTER_AGENT_PORT)),
                "{connection}"
            );
        }

        for connection in [
            ipv6_server,
            "Srvr=\"[::1]\";Ref=\"demo\"",
            "Srvr=tcp://[fe80::1]:1541;Ref=demo",
            "Srvr=::1;Ref=demo",
            "/S [::1]:1541\\demo",
        ] {
            let error = cluster_base(connection, "{user: admin}")
                .cluster_administration()
                .expect_err(connection);
            assert!(
                matches!(error, ClusterAdministrationError::ServerIsIpv6 { .. }),
                "{connection}: {error}"
            );
            assert!(
                error
                    .to_string()
                    .contains("rac and ras accept only a name or IPv4"),
                "{error}"
            );
        }

        assert_eq!(
            cluster_base("File=/tmp/ib", "{}").cluster_administration(),
            Err(ClusterAdministrationError::NoServer)
        );
    }

    /// IPv6 — то, что разбирается в адрес IPv6, голый адрес и запись со скобкой; опечатки
    /// с двумя двоеточиями и префиксом протокола адресом IPv6 не названы.
    #[test]
    fn an_ipv6_host_is_told_from_a_malformed_record() {
        for address in [
            "[::1]:1545",
            "[::1]",
            "::1",
            "fe80::1",
            "[::ffff:10.0.0.5]",
            "[::1",
        ] {
            assert!(names_an_ipv6_host(address), "{address}");
        }
        for address in [
            "srv",
            "srv:1545",
            "10.0.0.5:1540",
            "tcp://srv:1545",
            "srv::1545",
            "",
        ] {
            assert!(!names_an_ipv6_host(address), "{address}");
        }
    }
}
