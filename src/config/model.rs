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

    /// Settings of `push`: how sources reach the infobase
    #[serde(default)]
    #[serde(rename = "push", alias = "build")]
    pub build: BuildConfig,

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
    /// Connection string to the infobase: `File=…` or `Srvr=…;Ref=…`. Empty when the
    /// target is a standalone server, which `standalone` declares instead.
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

    /// Standalone server (`ibsrv`) reached through its SSH gate. Its presence declares
    /// the target kind; the runner never starts the server.
    #[serde(default)]
    pub standalone: Option<StandaloneConfig>,

    /// The cluster around a server infobase: the administration server address and the
    /// two administrator levels above the infobase user
    /// (`DEC.2026-09-21.THE-CLUSTER-SECTION-HOLDS-RAS-AND-TWO-ADMIN-LEVELS`). Validation
    /// checks its form; no operation reads it yet (`sessions` — #212, the runner's own
    /// `ras` — #213, `infobase create` in a cluster — #204).
    #[serde(default)]
    pub cluster: Option<InfobaseClusterConfig>,
}

/// The cluster section of a server infobase. Every key is optional: each operation asks
/// only for the level it needs, and the refusal names the level that is missing.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InfobaseClusterConfig {
    /// Administration server (`ras`) address as `host[:port]`; it goes to `rac` as is,
    /// so the port default (1545) stays with the platform.
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

/// A standalone server as the target: the runner attaches to its SSH gate and exchanges
/// files through a declared channel, never through a path it assumes to be shared.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct StandaloneConfig {
    /// `host:port` or `[v6]:port` of the server's SSH gate (`ibsrv --enable-ssh-gate`); the
    /// port is required — `ibsrv` listens on 1543 unless told otherwise.
    pub gate: String,

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
    /// The gate as `(host, port)`.
    pub fn gate_endpoint(&self) -> Result<(Host, u16), String> {
        ssh_endpoint(&self.gate)
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

    /// Who is assigned to an operation on this target, before any readiness check.
    ///
    /// An override names one provider and never falls back; a default is the matrix
    /// chain, from which the caller takes the first ready one.
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
                chain: capability::default_chain(operation, self.target_kind()),
            },
        }
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceSetPurpose {
    Configuration,
    Extension,
    ExternalDataProcessors,
    ExternalReports,
}

impl SourceSetPurpose {
    pub const fn is_external(self) -> bool {
        matches!(self, Self::ExternalDataProcessors | Self::ExternalReports)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildConfig {
    #[serde(default = "default_partial_load_threshold")]
    pub partial_load_threshold: usize,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            partial_load_threshold: default_partial_load_threshold(),
        }
    }
}

fn default_partial_load_threshold() -> usize {
    20
}

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

    /// Port the managed agent listens on. Managed mode; default `1543`.
    pub port: Option<u16>,

    /// Private host key for the managed agent. Absent: `/AgentSSHHostKeyAuto`.
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

/// Default SSH port of a Designer agent.
pub const DEFAULT_DESIGNER_AGENT_PORT: u16 = 1543;

/// The mode the keys of `tools.designer_agent` describe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignerAgentMode {
    /// The runner launches `1cv8 DESIGNER … /AgentMode` and owns its lifetime.
    Managed { port: u16 },
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
            None => Ok(DesignerAgentMode::Managed {
                port: self.port.unwrap_or(DEFAULT_DESIGNER_AGENT_PORT),
            }),
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
        ssh_endpoint, DesignerAgentConfig, DesignerAgentMode, Host, PlatformToolConfig,
        StandaloneConfig,
    };

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
}
