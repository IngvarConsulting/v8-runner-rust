use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "v8-runner",
    version,
    about = "Run 1C:Enterprise push, test, pull, convert, and launch workflows"
)]
pub struct Cli {
    /// Path to an existing YAML config file. Defaults to ./v8project.yaml
    #[arg(
        long,
        global = true,
        env = "V8TR_CONFIG",
        help_heading = "Global options"
    )]
    pub config: Option<String>,

    /// Print structured JSON envelopes instead of text output
    #[arg(long, global = true, help_heading = "Global options")]
    pub json_message: bool,

    /// Log level
    #[arg(long, global = true, default_value = "info",
          value_parser = ["error", "warn", "info", "debug", "trace"],
          help_heading = "Global options")]
    pub log_level: Option<String>,

    /// Clear log files before execution
    #[arg(long, global = true, help_heading = "Global options")]
    pub clean_before_execution: bool,

    /// Disable ANSI colors
    #[arg(long, global = true, help_heading = "Global options")]
    pub no_color: bool,

    /// Override working directory
    #[arg(long, global = true, help_heading = "Global options")]
    pub workdir: Option<String>,

    /// Infobase to work with: a name declared in v8project.local.yaml or a connection string; defaults to `origin`
    #[arg(
        long,
        global = true,
        value_name = "NAME|CONNECTION",
        help_heading = "Global options"
    )]
    pub infobase: Option<String>,

    /// Show the plan and locate the tools without dispatching anything: a command without a
    /// preview refuses the key instead of ignoring it
    #[arg(long, global = true, help_heading = "Global options")]
    pub dry_run: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print application version
    Version,
    /// Create a v8-runner project from an existing infobase
    #[command(name = "clone", alias = "bootstrap")]
    Bootstrap(BootstrapArgs),
    /// Prepare the project: generate configuration and autodetect source-sets
    #[command(name = "init")]
    ConfigInit(ConfigInitArgs),
    /// Previous spelling of `init`; hidden for one release cycle
    #[command(hide = true)]
    Config(ConfigArgs),
    /// Creating the infobase; parsed as `infobase create` and normalised here.
    #[command(skip)]
    Init,
    /// Download YaXUnit, Vanessa Automation, and client MCP tool assets
    Tools(ToolsArgs),
    /// Update extension security properties or manage installed extensions
    Extensions(ExtensionsArgs),
    /// Send configured source-sets to the infobase
    #[command(name = "push", alias = "build")]
    Build(BuildArgs),
    /// Upload a built package (.cf/.cfe) into the infobase
    #[command(name = "upload", alias = "load")]
    Load(LoadArgs),
    /// Run YaXUnit or Vanessa Automation tests, building first by default
    Test(TestArgs),
    /// Pull infobase state back into project files
    #[command(name = "pull", alias = "dump")]
    Dump(DumpArgs),
    /// Take the configuration out of the infobase as a package
    #[command(name = "download")]
    Download(InfobaseConfigurationExportArgs),
    /// Export configuration packages or a full DT snapshot from the configured infobase
    Infobase(InfobaseArgs),
    /// Convert configured source-sets between EDT and Designer file formats
    Convert(ConvertArgs),
    /// Export release artifacts via Designer batch commands
    #[command(name = "make", visible_alias = "artifacts")]
    Artifacts(ArtifactsArgs),
    /// Check the configuration with Designer or EDT
    #[command(name = "check", alias = "syntax")]
    Syntax(SyntaxArgs),
    /// Launch 1C application
    Launch(LaunchArgs),
    /// Publish the infobase on a web server with webinst, or delete the publication
    Publish(PublishArgs),
    /// Serve Model Context Protocol transports
    Mcp(McpArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct PublishArgs {
    /// Delete the publication named in infobase.web instead of creating it
    #[arg(long)]
    pub delete: bool,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct BootstrapArgs {
    /// Project directory to create. Defaults to the current directory.
    #[arg(long)]
    pub project_dir: Option<String>,

    /// Existing infobase connection string used as bootstrap source
    #[arg(long)]
    pub connection: String,

    /// 1C:Enterprise platform version written to project config
    #[arg(long)]
    pub platform_version: String,

    /// Local platform binary, bin directory, or installation root
    #[arg(long)]
    pub platform_path: Option<String>,

    /// Infobase user name stored in v8project.local.yaml
    #[arg(long)]
    pub user: Option<String>,

    /// Infobase password stored in v8project.local.yaml
    #[arg(long)]
    pub password: Option<String>,

    /// Source directory for the dumped main configuration
    #[arg(long, default_value = "src/configuration")]
    pub source_dir: String,

    /// Overwrite generated config/local config/source targets
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ToolsArgs {
    #[command(subcommand)]
    pub command: ToolsCommand,
}

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// Download a supported test or MCP helper tool from its latest GitHub release
    Download(ToolsDownloadArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ToolsDownloadArgs {
    #[command(subcommand)]
    pub command: ToolsDownloadCommand,
}

#[derive(Subcommand, Debug)]
pub enum ToolsDownloadCommand {
    /// Download YAxUnit extension assets or sources
    Yaxunit(ToolsDownloadExtensionArgs),
    /// Download Vanessa Automation Single external processor
    #[command(visible_alias = "vanessa-automation-single")]
    Vanessa(ToolsDownloadToolArgs),
    /// Download onec-client-mcp-devkit extension assets or sources
    #[command(name = "client-mcp", visible_alias = "client_mcp")]
    ClientMcp(ToolsDownloadExtensionArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ToolsDownloadExtensionArgs {
    /// Download extension sources instead of the release artifact
    #[arg(long)]
    pub sources: bool,

    /// Re-download managed targets created by tools download
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ToolsDownloadToolArgs {
    /// Re-download managed targets created by tools download
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Create a new config file and add detected sources
    Init(ConfigInitArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ConfigInitArgs {
    /// Overwrite an existing config file
    #[arg(long)]
    pub force: bool,

    /// Path to the generated YAML config file. Defaults to ./v8project.yaml
    #[arg(long)]
    pub output: Option<String>,

    /// Infobase connection string written to config
    #[arg(long)]
    pub connection: Option<String>,

    /// Source format to write
    #[arg(long, default_value = "auto", value_parser = ["auto", "designer", "edt"])]
    pub format: String,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct BuildArgs {
    /// Clear change cache and send everything anew
    #[arg(long = "full", alias = "full-rebuild")]
    pub full_rebuild: bool,

    /// Limit build to one source-set from v8project.yaml
    #[arg(long)]
    pub source_set: Option<String>,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct LoadArgs {
    /// Path to a built artifact (.cf/.cfe)
    #[arg(long)]
    pub path: String,

    /// Upload mode
    #[arg(
        long,
        default_value = "load",
        value_parser = clap::builder::PossibleValuesParser::new([
            clap::builder::PossibleValue::new("load"),
            clap::builder::PossibleValue::new("combine"),
            clap::builder::PossibleValue::new("update"),
            // Прежнее имя режима живёт один цикл выпуска и в справке не печатается.
            clap::builder::PossibleValue::new("merge").hide(true),
        ]),
    )]
    pub mode: String,

    /// Settings file used by --mode combine
    #[arg(long)]
    pub settings: Option<String>,

    /// Extension name required for .cfe artifacts
    #[arg(long)]
    pub extension: Option<String>,

    /// Vendor configuration name, required to ask whether a configuration is on support
    #[arg(long)]
    pub vendor_name: Option<String>,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ExtensionsArgs {
    /// Read or change the extension composition of the infobase.
    ///
    /// Without a subcommand, update security properties of the selected extensions.
    /// Without selectors, update all configured extension source-sets.
    #[command(subcommand)]
    pub command: Option<ExtensionsCommand>,

    /// Extension source-set name to update. Repeat to target multiple extensions.
    #[arg(long = "name")]
    pub names: Vec<String>,

    /// Installed extension's platform name; no matching source-set is required.
    /// Repeat or combine with --name to update only the explicitly selected targets.
    #[arg(long = "installed-name")]
    pub installed_names: Vec<String>,
}

impl ExtensionsArgs {
    /// Parent property options must not be silently ignored by a composition subcommand.
    /// Validate after parsing so global options remain legal around subcommands.
    pub fn validate_property_options(&self) -> Result<(), &'static str> {
        if self.command.is_some() && (!self.names.is_empty() || !self.installed_names.is_empty()) {
            return Err("extensions parent --name and --installed-name cannot be combined with a subcommand; place subcommand options after its name");
        }
        Ok(())
    }
}

#[derive(Subcommand, Debug)]
pub enum ExtensionsCommand {
    /// Report the extensions installed in the infobase
    List,
    /// Report one installed extension by its platform name
    Info(ExtensionNameArgs),
    /// Register a new extension in the infobase
    Create(ExtensionCreateArgs),
    /// Remove an extension from the infobase
    Delete(ExtensionNameArgs),
    /// Turn an installed extension on or off without removing it
    Activate(ExtensionActivateArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ExtensionNameArgs {
    /// Extension name as the platform knows it
    #[arg(long)]
    pub name: String,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ExtensionCreateArgs {
    /// Extension name as the platform will know it
    #[arg(long)]
    pub name: String,

    /// Name prefix for objects the extension adds
    #[arg(long = "name-prefix")]
    pub name_prefix: String,

    /// Synonym in `NStr()` format
    #[arg(long)]
    pub synonym: Option<String>,

    /// Extension purpose
    #[arg(long, value_parser = ["customization", "add-on", "patch"])]
    pub purpose: Option<String>,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ExtensionActivateArgs {
    /// Extension name as the platform knows it
    #[arg(long)]
    pub name: String,

    /// Target activity state
    #[arg(long, value_parser = ["yes", "no"])]
    pub active: String,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct TestArgs {
    #[arg(long, global = true)]
    pub full: bool,

    /// Run tests against the configured prepared infobase without sending sources first
    #[arg(long = "no-push", alias = "no-build", global = true)]
    pub no_build: bool,

    /// Client mode used for enterprise launch during test execution
    #[arg(long = "client-mode", value_parser = ["designer", "thin", "thick", "ordinary"])]
    pub client_mode: Option<String>,

    #[command(flatten)]
    pub launch: TestLaunchOptionsArgs,

    #[command(subcommand)]
    pub runner: TestRunner,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Command options")]
pub struct TestLaunchOptionsArgs {
    /// Enables `/UsePrivilegedMode`
    #[arg(long = "use-privileged-mode")]
    pub use_privileged_mode: bool,
    /// Additional raw launch arguments appended after typed launch keys
    #[arg(long = "raw-key")]
    pub raw_keys: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum TestRunner {
    /// Run YaXUnit tests
    Yaxunit(TestYaxunitArgs),
    /// Run Vanessa Automation feature scenarios
    Va(TestVaArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct TestYaxunitArgs {
    #[command(subcommand)]
    pub scope: TestScope,
}

#[derive(Args, Debug, Default)]
#[command(next_help_heading = "Vanessa Automation options")]
pub struct TestVaArgs {
    /// Feature name from the selected Vanessa profile to run. Repeat to run multiple features.
    #[arg(long = "feature")]
    pub features_to_run: Vec<String>,

    /// Tag expression to include. Repeat to pass multiple include tags.
    #[arg(long = "filter-tag")]
    pub filter_tags: Vec<String>,

    /// Tag expression to exclude. Repeat to pass multiple exclude tags.
    #[arg(long = "ignore-tag")]
    pub ignore_tags: Vec<String>,

    /// Scenario name filter. Repeat to pass multiple scenario filters.
    #[arg(long = "scenario-filter")]
    pub scenario_filter: Vec<String>,
}

impl TestVaArgs {
    pub fn has_profile_overrides(&self) -> bool {
        !self.features_to_run.is_empty()
            || !self.filter_tags.is_empty()
            || !self.ignore_tags.is_empty()
            || !self.scenario_filter.is_empty()
    }
}

#[derive(Subcommand, Debug)]
pub enum TestScope {
    /// Run all tests
    All,
    /// Run tests for a specific module
    Module {
        /// Module name
        name: String,
    },
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct DumpArgs {
    /// Dump mode
    #[arg(long, value_parser = ["full", "incremental", "partial"])]
    pub mode: String,

    /// Source set name
    #[arg(long)]
    pub source_set: Option<String>,

    /// Extension name
    #[arg(long)]
    pub extension: Option<String>,

    /// Objects for partial dump. Use canonical TYPE:NAME selectors; legacy TYPE.NAME selectors are accepted for compatibility.
    #[arg(long = "object")]
    pub objects: Vec<String>,

    /// Replace the target directory even when it holds work version control cannot give back
    #[arg(long = "force", alias = "discard-uncommitted")]
    pub discard_uncommitted: bool,
}

#[derive(Args, Debug)]
pub struct InfobaseArgs {
    #[command(subcommand)]
    pub command: InfobaseCommand,
}

#[derive(Subcommand, Debug)]
pub enum InfobaseCommand {
    /// Create the infobase and the EDT workspace declared by the project
    Create,
    /// Previous spelling of `download`; hidden for one release cycle
    #[command(hide = true)]
    Configuration(InfobaseConfigurationArgs),
    /// Export the complete infobase to a DT transfer file (not a backup)
    Dump(InfobaseDumpArgs),
    /// Load the complete infobase from a DT transfer file, discarding current data
    Restore(InfobaseRestoreArgs),
}

#[derive(Args, Debug)]
pub struct InfobaseConfigurationArgs {
    #[command(subcommand)]
    pub command: InfobaseConfigurationCommand,
}

#[derive(Subcommand, Debug)]
pub enum InfobaseConfigurationCommand {
    /// Export the selected configuration state to a package file
    Export(InfobaseConfigurationExportArgs),
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct InfobaseConfigurationExportArgs {
    /// Configuration state to export
    #[arg(long, value_parser = ["working", "database"])]
    pub state: String,

    /// Extension name; omit to export the main configuration
    #[arg(long)]
    pub extension: Option<String>,

    /// Final CF/CFE output path
    #[arg(long)]
    pub output: String,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct InfobaseDumpArgs {
    /// Final DT output path; a DT transfer image is not a database backup
    #[arg(long)]
    pub output: String,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct InfobaseRestoreArgs {
    /// Source DT transfer file
    #[arg(long)]
    pub input: String,

    /// Create the target infobase; refuses when it already exists
    #[arg(long, conflicts_with = "replace")]
    pub create: bool,

    /// Discard the data of the existing target infobase; refuses when it is absent
    #[arg(long, conflicts_with = "create")]
    pub replace: bool,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ConvertArgs {
    /// Limit conversion to one source-set from v8project.yaml
    #[arg(long)]
    pub source_set: Option<String>,

    /// Target root for converted source-set layout. Defaults to workPath/convert/out
    #[arg(long)]
    pub output: Option<String>,

    /// Replace the target directory even when it holds work version control cannot give back
    #[arg(long = "force", alias = "discard-uncommitted")]
    pub discard_uncommitted: bool,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct ArtifactsArgs {
    /// Final output path (.cf/.cfe file or publish directory for external artifacts)
    #[arg(long)]
    pub output: String,

    /// Optional source set name used to disambiguate repository context
    #[arg(long)]
    pub source_set: Option<String>,

    /// Extension name in the infobase for cfe export
    #[arg(long)]
    pub extension: Option<String>,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Ветку выбирает format проекта: DESIGNER — /CheckConfig, EDT — проверка проекта.\nБез единого ключа режима выполняется профиль по умолчанию: --thin-client --server\n--unreference-procedures --handlers-existence --empty-handlers --extended-modules-check."
)]
pub struct SyntaxArgs {
    /// Режимы `/CheckConfig`. Без единого ключа выполняется профиль по умолчанию.
    #[command(flatten)]
    pub modes: DesignerConfigSyntaxArgs,
    /// EDT project names
    #[arg(long = "project", help_heading = "Command options")]
    pub projects: Vec<String>,
    /// Прежние имена: приняты один цикл, в справке их нет.
    #[command(subcommand)]
    pub target: Option<SyntaxTarget>,
}

impl SyntaxArgs {
    /// Ключи самой команды рядом с прежним именем не исполняются, поэтому отвергаются.
    pub fn keys_next_to_a_previous_name(&self) -> Option<&'static str> {
        (self.target.is_some()
            && (self.modes != DesignerConfigSyntaxArgs::default() || !self.projects.is_empty()))
        .then_some("check keys cannot be combined with a subcommand; place the keys after its name")
    }
}

#[derive(Subcommand, Debug)]
pub enum SyntaxTarget {
    /// Check configuration via Designer CheckConfig
    #[command(hide = true)]
    DesignerConfig(DesignerConfigSyntaxArgs),
    /// Check modules via Designer CheckConfig module modes
    #[command(hide = true)]
    DesignerModules(DesignerModulesSyntaxArgs),
    /// Check via EDT validate
    #[command(hide = true)]
    Edt {
        /// EDT project names
        #[arg(long = "project")]
        projects: Vec<String>,
    },
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Command options")]
pub struct LaunchArgs {
    /// Launch mode; `web` opens infobase.web.url in the browser
    #[arg(value_name = "MODE", value_parser = ["designer", "thin", "thick", "ordinary", "mcp", "web"])]
    pub target: String,

    /// Optional client-side MCP scenario to start with the MCP server
    #[arg(value_name = "MCP_SCENARIO", value_parser = ["va"])]
    pub mcp_scenario: Option<String>,

    /// 1C client mode for `launch mcp`
    #[arg(long = "mode", value_parser = ["thin", "thick", "ordinary"])]
    pub mcp_mode: Option<String>,

    #[command(flatten)]
    pub launch: DirectLaunchOptionsArgs,

    /// Which address opens the base: `web` for infobase.web.url, `connection` for
    /// infobase.connection. Thin client only; the default follows the target kind
    #[arg(long = "via", value_parser = ["web", "connection"])]
    pub via: Option<String>,

    /// JSON config path for onec-client-mcp-devkit `/C runMcp=<FILE>`
    #[arg(long = "mcp-config")]
    pub mcp_config: Option<String>,

    /// Port override for onec-client-mcp-devkit `/C ...;mcpPort=<PORT>`
    #[arg(long = "mcp-port")]
    pub mcp_port: Option<u16>,

    /// Wait until the client MCP HTTP endpoint is initialized and tools/list succeeds
    #[arg(long = "wait-ready")]
    pub wait_ready: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Command options")]
pub struct LaunchOptionsArgs {
    /// Value for `/C`
    #[arg(long = "c")]
    pub c: Option<String>,
    /// Value for `/Execute`
    #[arg(long)]
    pub execute: Option<String>,
    /// Enables `/UsePrivilegedMode`
    #[arg(long = "use-privileged-mode")]
    pub use_privileged_mode: bool,
    /// User-provided `/Out` path allowed only for direct launch
    #[arg(long)]
    pub output: Option<String>,
    /// Additional raw launch arguments appended after typed launch keys
    #[arg(long = "raw-key")]
    pub raw_keys: Vec<String>,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Command options")]
pub struct DirectLaunchOptionsArgs {
    #[command(flatten)]
    pub common: LaunchOptionsArgs,
    /// Capture client stderr to this path while waiting for an external EPF to exit
    #[arg(long = "stderr-output")]
    pub stderr_output: Option<String>,
    /// Wait for a direct external EPF launch to exit
    #[arg(long = "wait-for-exit")]
    pub wait_for_exit: bool,
    /// Maximum wait time in milliseconds for --wait-for-exit
    #[arg(long = "wait-timeout-ms")]
    pub wait_timeout_ms: Option<u64>,
}

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// Serve an MCP transport
    Serve(McpServeArgs),
}

#[derive(Args, Debug)]
pub struct McpServeArgs {
    #[command(subcommand)]
    pub transport: McpServeTransport,
}

#[derive(Subcommand, Debug)]
pub enum McpServeTransport {
    /// Serve MCP over stdio
    Stdio,
    /// Serve MCP over streamable HTTP
    Http,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Command options")]
pub struct DesignerConfigSyntaxArgs {
    #[arg(long)]
    pub config_log_integrity: bool,
    #[arg(long)]
    pub incorrect_references: bool,
    #[arg(long)]
    pub thin_client: bool,
    #[arg(long)]
    pub web_client: bool,
    #[arg(long)]
    pub mobile_client: bool,
    #[arg(long)]
    pub server: bool,
    #[arg(long)]
    pub external_connection: bool,
    #[arg(long)]
    pub external_connection_server: bool,
    #[arg(long)]
    pub mobile_app_client: bool,
    #[arg(long)]
    pub mobile_app_server: bool,
    #[arg(long)]
    pub thick_client_managed_application: bool,
    #[arg(long)]
    pub thick_client_server_managed_application: bool,
    #[arg(long)]
    pub thick_client_ordinary_application: bool,
    #[arg(long)]
    pub thick_client_server_ordinary_application: bool,
    #[arg(long)]
    pub mobile_client_digi_sign: bool,
    #[arg(long)]
    pub distributive_modules: bool,
    #[arg(long)]
    pub unreference_procedures: bool,
    #[arg(long)]
    pub handlers_existence: bool,
    #[arg(long)]
    pub empty_handlers: bool,
    #[arg(long)]
    pub extended_modules_check: bool,
    #[arg(long, requires = "extended_modules_check")]
    pub check_use_synchronous_calls: bool,
    #[arg(long, requires = "extended_modules_check")]
    pub check_use_modality: bool,
    #[arg(long)]
    pub unsupported_functional: bool,
    #[arg(long, conflicts_with = "all_extensions")]
    pub extension: Option<String>,
    #[arg(long)]
    pub all_extensions: bool,
}

#[derive(Args, Debug, Clone)]
#[command(next_help_heading = "Command options")]
pub struct DesignerModulesSyntaxArgs {
    #[arg(long)]
    pub thin_client: bool,
    #[arg(long)]
    pub web_client: bool,
    #[arg(long)]
    pub server: bool,
    #[arg(long)]
    pub external_connection: bool,
    #[arg(long)]
    pub thick_client_ordinary_application: bool,
    #[arg(long)]
    pub mobile_app_client: bool,
    #[arg(long)]
    pub mobile_app_server: bool,
    #[arg(long)]
    pub mobile_client: bool,
    #[arg(long)]
    pub extended_modules_check: bool,
    #[arg(long, conflicts_with = "all_extensions")]
    pub extension: Option<String>,
    #[arg(long)]
    pub all_extensions: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactsArgs, Cli, Command, ConvertArgs, DirectLaunchOptionsArgs, ExtensionsArgs,
        InfobaseArgs, InfobaseCommand, LaunchArgs, LoadArgs, McpCommand, McpServeTransport,
        SyntaxTarget, TestLaunchOptionsArgs, TestRunner, TestScope,
    };
    use clap::Parser;

    #[test]
    fn syntax_config_extension_conflicts_with_all_extensions() {
        let result = Cli::try_parse_from([
            "v8-runner",
            "syntax",
            "designer-config",
            "--extension",
            "Ext",
            "--all-extensions",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn syntax_config_sync_calls_require_extended_modules_check() {
        let result = Cli::try_parse_from([
            "v8-runner",
            "syntax",
            "designer-config",
            "--check-use-synchronous-calls",
        ]);

        assert!(result.is_err());
    }

    /// Имя `init` перешло к подготовке проекта, а создание базы живёт под
    /// `infobase create`: это единственное имя словаря без синонима.
    #[test]
    fn init_prepares_the_project_and_the_infobase_is_created_by_its_own_command() {
        let cli = Cli::try_parse_from(["v8-runner", "init"]).expect("parse");
        assert!(matches!(cli.command, Command::ConfigInit(_)));

        let cli = Cli::try_parse_from(["v8-runner", "infobase", "create"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Infobase(InfobaseArgs {
                command: InfobaseCommand::Create,
            })
        ));
    }

    /// Прежние имена принимаются и в справке не печатаются.
    #[test]
    fn a_previous_command_name_is_accepted_as_a_hidden_synonym() {
        for (previous, expected) in [
            ("build", "push"),
            ("dump", "pull"),
            ("load", "upload"),
            ("syntax", "check"),
            ("bootstrap", "clone"),
        ] {
            let cli = Cli::try_parse_from(["v8-runner", previous, "--help"]);
            // `--help` прерывает разбор, но имя уже разрешено: ошибка печатает новое имя.
            let rendered = cli.expect_err("help exits with an error kind").to_string();
            assert!(rendered.contains(expected), "{previous}: {rendered}");
            assert!(
                !rendered.contains(&format!("v8-runner {previous}")),
                "{rendered}"
            );
        }
    }

    #[test]
    fn parses_version_command() {
        let cli = Cli::try_parse_from(["v8-runner", "version"]).expect("parse");
        assert!(matches!(cli.command, Command::Version));
    }

    #[test]
    fn parses_extensions_command_with_names() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "extensions",
            "--name",
            "client_mcp",
            "--name",
            "tests",
        ])
        .expect("parse");

        match cli.command {
            Command::Extensions(ExtensionsArgs {
                names,
                command,
                installed_names,
            }) => {
                assert!(command.is_none());
                assert_eq!(names, vec!["client_mcp", "tests"]);
                assert!(installed_names.is_empty());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_explicit_extension_targets_and_preview() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "extensions",
            "--installed-name",
            "YAXUNIT",
            "--name",
            "tests",
            "--installed-name",
            "Другое",
            "--dry-run",
        ])
        .expect("parse");
        let preview = cli.dry_run;
        let Command::Extensions(args) = cli.command else {
            panic!("unexpected command");
        };
        assert_eq!(args.names, ["tests"]);
        assert_eq!(args.installed_names, ["YAXUNIT", "Другое"]);
        assert!(preview);
        assert!(args.command.is_none());
    }

    #[test]
    fn extension_parent_options_cannot_be_ignored_by_subcommands() {
        for arguments in [
            vec!["--name", "tests", "list"],
            vec!["--installed-name", "YAXUNIT", "list"],
            vec!["--installed-name", "YAXUNIT", "delete", "--name", "Other"],
        ] {
            let cli = Cli::try_parse_from(["v8-runner", "extensions"].into_iter().chain(arguments))
                .expect("parse before semantic validation");
            let Command::Extensions(args) = cli.command else {
                panic!("unexpected command");
            };
            assert!(args.validate_property_options().is_err());
        }
        // Ключ превью стал глобальным: перед подкомандой он значит то же, что после неё.
        let cli = Cli::try_parse_from([
            "v8-runner",
            "extensions",
            "--dry-run",
            "delete",
            "--name",
            "Other",
        ])
        .expect("parse");
        assert!(cli.dry_run);
        let Command::Extensions(args) = cli.command else {
            panic!("unexpected command");
        };
        assert!(args.validate_property_options().is_ok());
        // Global transport/config options still work before and after a subcommand.
        let cli = Cli::try_parse_from([
            "v8-runner",
            "extensions",
            "--json-message",
            "list",
            "--config",
            "v8project.yaml",
            "--dry-run",
        ])
        .expect("global options with subcommand");
        let Command::Extensions(args) = cli.command else {
            panic!("unexpected command");
        };
        assert!(args.validate_property_options().is_ok());
    }

    #[test]
    fn parses_load_command_with_default_mode() {
        let cli = Cli::try_parse_from(["v8-runner", "load", "--path", "dist/main.cf"])
            .expect("parse load");

        match cli.command {
            Command::Load(LoadArgs {
                path,
                mode,
                settings,
                extension,
                vendor_name,
            }) => {
                assert_eq!(path, "dist/main.cf");
                assert_eq!(mode, "load");
                assert!(settings.is_none());
                assert!(extension.is_none());
                assert!(vendor_name.is_none());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_load_command_with_merge_mode() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "load",
            "--path",
            "dist/ext.cfe",
            "--mode",
            "merge",
            "--settings",
            "merge.xml",
            "--extension",
            "SalesAddon",
        ])
        .expect("parse load merge");

        match cli.command {
            Command::Load(LoadArgs {
                path,
                mode,
                settings,
                extension,
                vendor_name: _,
            }) => {
                assert_eq!(path, "dist/ext.cfe");
                assert_eq!(mode, "merge");
                assert_eq!(settings.as_deref(), Some("merge.xml"));
                assert_eq!(extension.as_deref(), Some("SalesAddon"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_test_yaxunit_module_command() {
        let cli = Cli::try_parse_from(["v8-runner", "test", "yaxunit", "module", "Foo"])
            .expect("parse test yaxunit");

        match cli.command {
            Command::Test(args) => {
                assert!(!args.full);
                assert_eq!(args.launch.raw_keys, Vec::<String>::new());
                match args.runner {
                    TestRunner::Yaxunit(yaxunit) => {
                        assert!(
                            matches!(yaxunit.scope, TestScope::Module { name } if name == "Foo")
                        );
                    }
                    _ => panic!("unexpected test runner"),
                }
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_test_va_command() {
        let cli = Cli::try_parse_from(["v8-runner", "test", "va"]).expect("parse test va");

        match cli.command {
            Command::Test(args) => {
                assert!(matches!(args.runner, TestRunner::Va(_)));
                assert_eq!(args.launch, TestLaunchOptionsArgs::default());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_test_va_filter_options() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "test",
            "va",
            "--feature",
            "login",
            "--filter-tag",
            "@smoke",
            "--ignore-tag",
            "@draft",
            "--scenario-filter",
            "Проверка логина",
        ])
        .expect("parse test va filters");

        match cli.command {
            Command::Test(args) => match args.runner {
                TestRunner::Va(va) => {
                    assert_eq!(va.features_to_run, ["login"]);
                    assert_eq!(va.filter_tags, ["@smoke"]);
                    assert_eq!(va.ignore_tags, ["@draft"]);
                    assert_eq!(va.scenario_filter, ["Проверка логина"]);
                }
                _ => panic!("unexpected test runner"),
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_test_command_with_launch_options() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "test",
            "--use-privileged-mode",
            "--raw-key",
            "/WA-",
            "yaxunit",
            "all",
        ])
        .expect("parse test");

        match cli.command {
            Command::Test(args) => {
                assert_eq!(
                    args.launch,
                    TestLaunchOptionsArgs {
                        use_privileged_mode: true,
                        raw_keys: vec!["/WA-".to_owned()],
                    }
                );
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_launch_command_with_typed_and_raw_keys() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "launch",
            "ordinary",
            "--c",
            "DoWork",
            "--execute",
            "tool.epf",
            "--use-privileged-mode",
            "--output",
            "launch.log",
            "--raw-key",
            "/WA-",
            "--raw-key",
            "/DisplayAllFunctions",
        ])
        .expect("parse launch");

        match cli.command {
            Command::Launch(LaunchArgs {
                target,
                launch,
                mcp_scenario,
                mcp_mode,
                mcp_config,
                mcp_port,
                wait_ready,
                via: _,
            }) => {
                assert_eq!(target, "ordinary");
                assert_eq!(launch.common.c.as_deref(), Some("DoWork"));
                assert_eq!(launch.common.execute.as_deref(), Some("tool.epf"));
                assert!(launch.common.use_privileged_mode);
                assert_eq!(launch.common.output.as_deref(), Some("launch.log"));
                assert_eq!(launch.common.raw_keys, vec!["/WA-", "/DisplayAllFunctions"]);
                assert_eq!(mcp_scenario, None);
                assert_eq!(mcp_mode, None);
                assert_eq!(mcp_config, None);
                assert_eq!(mcp_port, None);
                assert!(!wait_ready);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_global_json_message_flag() {
        let cli = Cli::try_parse_from(["v8-runner", "--json-message", "build"]).expect("parse");
        assert!(cli.json_message);
    }

    #[test]
    fn parses_config_init_output_override() {
        let cli = Cli::try_parse_from(["v8-runner", "config", "init", "--output", "custom.yaml"])
            .expect("parse config init");

        match cli.command {
            Command::Config(config) => match config.command {
                super::ConfigCommand::Init(args) => {
                    assert_eq!(args.output.as_deref(), Some("custom.yaml"));
                }
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_launch_command_with_positional_mode() {
        let cli = Cli::try_parse_from(["v8-runner", "launch", "designer"]).expect("parse launch");

        match cli.command {
            Command::Launch(LaunchArgs {
                target,
                launch,
                mcp_scenario,
                mcp_mode,
                mcp_config,
                mcp_port,
                wait_ready,
                via: _,
            }) => {
                assert_eq!(target, "designer");
                assert_eq!(launch, DirectLaunchOptionsArgs::default());
                assert_eq!(mcp_scenario, None);
                assert_eq!(mcp_mode, None);
                assert_eq!(mcp_config, None);
                assert_eq!(mcp_port, None);
                assert!(!wait_ready);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_launch_dry_run_flag() {
        let cli = Cli::try_parse_from(["v8-runner", "launch", "thin", "--dry-run"])
            .expect("parse launch preview");

        let preview = cli.dry_run;
        match cli.command {
            Command::Launch(args) => {
                assert_eq!(args.target, "thin");
                assert!(preview);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_launch_mcp_command_with_mcp_options() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "launch",
            "mcp",
            "va",
            "--mode",
            "ordinary",
            "--mcp-config",
            "mcp-conf.json",
            "--mcp-port",
            "9876",
            "--wait-ready",
        ])
        .expect("parse launch");

        match cli.command {
            Command::Launch(LaunchArgs {
                target,
                launch,
                mcp_scenario,
                mcp_mode,
                mcp_config,
                mcp_port,
                wait_ready,
                via: _,
            }) => {
                assert_eq!(target, "mcp");
                assert_eq!(launch, DirectLaunchOptionsArgs::default());
                assert_eq!(mcp_scenario.as_deref(), Some("va"));
                assert_eq!(mcp_mode.as_deref(), Some("ordinary"));
                assert_eq!(mcp_config.as_deref(), Some("mcp-conf.json"));
                assert_eq!(mcp_port, Some(9876));
                assert!(wait_ready);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn syntax_modules_all_extensions_conflicts_with_extension() {
        let result = Cli::try_parse_from([
            "v8-runner",
            "syntax",
            "designer-modules",
            "--server",
            "--extension",
            "Ext",
            "--all-extensions",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn syntax_config_accepts_zero_mode_flags() {
        let cli = Cli::try_parse_from(["v8-runner", "syntax", "designer-config"])
            .expect("parse syntax config");

        match cli.command {
            Command::Syntax(args) => match args.target.expect("hidden synonym") {
                SyntaxTarget::DesignerConfig(config) => {
                    assert!(!config.server);
                    assert!(!config.all_extensions);
                }
                _ => panic!("unexpected syntax target"),
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_mcp_stdio_command() {
        let cli =
            Cli::try_parse_from(["v8-runner", "mcp", "serve", "stdio"]).expect("parse mcp stdio");

        match cli.command {
            Command::Mcp(args) => match args.command {
                McpCommand::Serve(serve) => {
                    assert!(matches!(serve.transport, McpServeTransport::Stdio));
                }
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_mcp_http_command() {
        let cli =
            Cli::try_parse_from(["v8-runner", "mcp", "serve", "http"]).expect("parse mcp http");

        match cli.command {
            Command::Mcp(args) => match args.command {
                McpCommand::Serve(serve) => {
                    assert!(matches!(serve.transport, McpServeTransport::Http));
                }
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_make_cf_command() {
        let cli = Cli::try_parse_from(["v8-runner", "make", "--output", "dist/main.cf"])
            .expect("parse make");

        match cli.command {
            Command::Artifacts(ArtifactsArgs {
                output,
                source_set,
                extension,
            }) => {
                assert_eq!(output, "dist/main.cf");
                assert!(source_set.is_none());
                assert!(extension.is_none());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_convert_without_source_set() {
        let cli = Cli::try_parse_from(["v8-runner", "convert"]).expect("parse convert");

        match cli.command {
            Command::Convert(ConvertArgs {
                source_set,
                output,
                discard_uncommitted,
            }) => {
                assert!(!discard_uncommitted);
                assert!(source_set.is_none());
                assert!(output.is_none());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_convert_with_source_set() {
        let cli = Cli::try_parse_from(["v8-runner", "convert", "--source-set", "ext-sales"])
            .expect("parse convert");

        match cli.command {
            Command::Convert(ConvertArgs {
                source_set,
                output,
                discard_uncommitted,
            }) => {
                assert!(!discard_uncommitted);
                assert_eq!(source_set.as_deref(), Some("ext-sales"));
                assert!(output.is_none());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_convert_with_output_root() {
        let cli = Cli::try_parse_from(["v8-runner", "convert", "--output", "tests/fixtures/edt"])
            .expect("parse convert");

        match cli.command {
            Command::Convert(ConvertArgs {
                source_set,
                output,
                discard_uncommitted,
            }) => {
                assert!(!discard_uncommitted);
                assert!(source_set.is_none());
                assert_eq!(output.as_deref(), Some("tests/fixtures/edt"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_make_cfe_command_with_extension_and_source_set() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "make",
            "--output",
            "dist/ext.cfe",
            "--source-set",
            "ext-sales",
            "--extension",
            "SalesAddon",
        ])
        .expect("parse make");

        match cli.command {
            Command::Artifacts(ArtifactsArgs {
                output,
                source_set,
                extension,
            }) => {
                assert_eq!(output, "dist/ext.cfe");
                assert_eq!(source_set.as_deref(), Some("ext-sales"));
                assert_eq!(extension.as_deref(), Some("SalesAddon"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_artifacts_alias_command() {
        let cli = Cli::try_parse_from(["v8-runner", "artifacts", "--output", "dist/main.cf"])
            .expect("parse artifacts alias");

        assert!(matches!(cli.command, Command::Artifacts(_)));
    }

    #[test]
    fn parses_infobase_configuration_export_contract() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "infobase",
            "configuration",
            "export",
            "--state",
            "database",
            "--extension",
            "SalesAddon",
            "--output",
            "dist/sales.cfe",
        ])
        .expect("parse configuration export");

        match cli.command {
            Command::Infobase(args) => match args.command {
                super::InfobaseCommand::Configuration(configuration) => {
                    match configuration.command {
                        super::InfobaseConfigurationCommand::Export(export) => {
                            assert_eq!(export.state, "database");
                            assert_eq!(export.extension.as_deref(), Some("SalesAddon"));
                            assert_eq!(export.output, "dist/sales.cfe");
                        }
                    }
                }
                _ => panic!("unexpected infobase command"),
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_infobase_dump_contract() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "infobase",
            "dump",
            "--output",
            "dist/snapshot.dt",
        ])
        .expect("parse infobase dump");

        match cli.command {
            Command::Infobase(args) => match args.command {
                super::InfobaseCommand::Dump(dump) => {
                    assert_eq!(dump.output, "dist/snapshot.dt");
                }
                _ => panic!("unexpected infobase command"),
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_infobase_restore_contract() {
        let cli = Cli::try_parse_from([
            "v8-runner",
            "infobase",
            "restore",
            "--input",
            "dist/snapshot.dt",
            "--replace",
            "--dry-run",
        ])
        .expect("parse infobase restore");

        let preview = cli.dry_run;
        match cli.command {
            Command::Infobase(args) => match args.command {
                super::InfobaseCommand::Restore(restore) => {
                    assert_eq!(restore.input, "dist/snapshot.dt");
                    assert!(!restore.create);
                    assert!(restore.replace);
                    assert!(preview);
                }
                _ => panic!("unexpected infobase command"),
            },
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn infobase_restore_rejects_both_target_modes_at_once() {
        let result = Cli::try_parse_from([
            "v8-runner",
            "infobase",
            "restore",
            "--input",
            "dist/snapshot.dt",
            "--create",
            "--replace",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn infobase_restore_requires_an_input() {
        let result = Cli::try_parse_from(["v8-runner", "infobase", "restore", "--replace"]);

        assert!(result.is_err());
    }

    #[test]
    fn infobase_configuration_export_rejects_unknown_state() {
        let result = Cli::try_parse_from([
            "v8-runner",
            "infobase",
            "configuration",
            "export",
            "--state",
            "current",
            "--output",
            "dist/main.cf",
        ]);

        assert!(result.is_err());
    }
}
