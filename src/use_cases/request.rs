use crate::domain::artifacts::{CFE_RUNNER_ID, CF_RUNNER_ID, EPF_RUNNER_ID, ERF_RUNNER_ID};
use crate::domain::execution::ExecutionTimeouts;
use crate::domain::launch::LaunchVia;
use crate::domain::load::LoadMode;
use crate::domain::runner::{
    ExecutionPolicy, LaunchClientModeRequest, LaunchOptions, RunnerKind, RunnerOutputFormat,
    RunnerProfile, ScenarioExecutionRequest,
};
use crate::domain::test::TEST_RUNNER_ID;
use crate::domain::tools_download::{ToolDownloadTarget, ToolExtensionInstallMode};
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Transport-neutral request for the `build` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// Forces a full rebuild instead of change-based execution.
    pub full_rebuild: bool,
    /// Optional source-set selector. When absent, all configured source-sets are built.
    pub source_set: Option<String>,
    /// Plan every step and locate the platform without dispatching it.
    pub dry_run: bool,
}

/// Transport-neutral request for the `tools download` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsDownloadRequest {
    /// Canonical path to the primary project config that may be updated.
    pub config_path: std::path::PathBuf,
    /// Selected tool to download.
    pub target: ToolDownloadTarget,
    /// Installation mode for extension tools.
    pub extensions: ToolExtensionInstallMode,
    /// Allows replacing existing downloaded paths.
    pub force: bool,
}

/// Transport-neutral request for the `load` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    /// Requested mode for artifact application.
    pub mode: LoadMode,
    /// Path to artifact file.
    pub artifact_path: String,
    /// Optional merge settings file.
    pub settings_path: Option<String>,
    /// Optional extension target.
    pub extension: Option<String>,
    /// Name of the vendor configuration to compare a configuration against.
    ///
    /// The platform refuses to compare a configuration with its vendor counterpart unless the
    /// counterpart is named, so without this the support state of a configuration cannot be
    /// established at all. An extension needs no such input: `extension` already names it.
    pub vendor_name: Option<String>,
    /// Resolve the artifact and locate Designer without probing or applying anything.
    pub dry_run: bool,
}

/// Transport-neutral request for the `test` use case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestBuildPolicy {
    BuildFirst,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRequest {
    /// Shared runner execution block reused by future test/package scenarios.
    pub execution: ScenarioExecutionRequest,
    /// When `true`, the use case may request a full build before test execution.
    pub full: bool,
    /// Whether the test coordinator prepares the infobase from configured sources.
    pub build_policy: TestBuildPolicy,
    /// Selected test scope. Module targets require a non-empty module name.
    pub scope: TestScopeRequest,
}

impl TestRequest {
    /// Default YaXUnit execution contract for the current test flow.
    /// Note: only `timeouts.total_ms` is wired into runtime today; the policy
    /// flags are retained as part of the shared OCP-friendly contract.
    pub fn default_execution() -> ScenarioExecutionRequest {
        ScenarioExecutionRequest {
            profile: RunnerProfile {
                id: TEST_RUNNER_ID.to_owned(),
                kind: RunnerKind::YaXUnit,
                output_formats: vec![
                    RunnerOutputFormat::JunitXml,
                    RunnerOutputFormat::PlainTextLog,
                ],
                backend_hint: Some("enterprise".to_owned()),
            },
            client_mode: Some(LaunchClientModeRequest::Thin),
            timeouts: ExecutionTimeouts::default(),
            policy: ExecutionPolicy {
                retain_artifacts_on_failure: true,
                retain_artifacts_on_success: false,
            },
            launch: LaunchOptions::default(),
        }
    }
}

pub(crate) fn effective_test_timeouts(
    legacy_total_seconds: u64,
    runner_timeouts: &ExecutionTimeouts,
) -> ExecutionTimeouts {
    let mut timeouts = runner_timeouts.clone();
    if timeouts.total_ms.is_none() {
        timeouts.total_ms = Some(legacy_total_seconds.saturating_mul(1_000));
    }
    timeouts
}

/// Transport-neutral test scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestScopeRequest {
    All,
    /// Runs a single module test target.
    Module {
        name: String,
    },
}

/// Transport-neutral dump mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpModeRequest {
    Full,
    Incremental,
    Partial,
}

/// Transport-neutral request for the `dump` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpRequest {
    /// Requested dump mode. `Partial` requires at least one object selector.
    pub mode: DumpModeRequest,
    /// Optional source-set selector. Required when multiple candidates are available.
    pub source_set: Option<String>,
    /// Optional extension selector for extension dumps.
    pub extension: Option<String>,
    /// Requested object filters for `Partial` dump mode.
    pub objects: Vec<String>,
    /// Resolve the target and locate the platform without dumping anything.
    pub dry_run: bool,
    /// Replace the target directory although it holds work version control cannot
    /// give back. Only a human can grant this; automated transports never do.
    pub discard_uncommitted: bool,
}

/// Transport-neutral convert scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertScopeRequest {
    All,
    SourceSet { name: String },
}

/// Transport-neutral request for the `convert` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertRequest {
    /// Requested convert scope.
    pub scope: ConvertScopeRequest,
    /// Optional user-facing target root for converted source-set layout.
    pub output_root: Option<String>,
    /// Resolve, validate and locate the EDT CLI without converting anything.
    pub dry_run: bool,
    /// Replace the target directory although it holds work version control cannot
    /// give back. Only a human can grant this; automated transports never do.
    pub discard_uncommitted: bool,
}

/// Transport-neutral artifact export mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactsModeRequest {
    ConfigurationCf,
    ExtensionCfe,
    ExternalDataProcessorEpf,
    ExternalReportErf,
}

/// Transport-neutral request for the `artifacts` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactsRequest {
    /// Shared runner execution block for packaging-like scenarios.
    pub execution: ScenarioExecutionRequest,
    /// Requested artifact export mode.
    pub mode: ArtifactsModeRequest,
    /// Final output file path provided by the caller.
    pub output_path: String,
    /// Optional source-set selector used to disambiguate repo context.
    pub source_set: Option<String>,
    /// Requested extension name in the infobase for `-Extension`.
    pub extension: Option<String>,
    /// Resolve the target and locate Designer without building or publishing anything.
    pub dry_run: bool,
}

impl ArtifactsRequest {
    pub fn default_execution(mode: ArtifactsModeRequest) -> ScenarioExecutionRequest {
        let (id, kind) = match mode {
            ArtifactsModeRequest::ConfigurationCf => (CF_RUNNER_ID, RunnerKind::Cf),
            ArtifactsModeRequest::ExtensionCfe => (CFE_RUNNER_ID, RunnerKind::Cfe),
            ArtifactsModeRequest::ExternalDataProcessorEpf => (EPF_RUNNER_ID, RunnerKind::Epf),
            ArtifactsModeRequest::ExternalReportErf => (ERF_RUNNER_ID, RunnerKind::Erf),
        };

        ScenarioExecutionRequest {
            profile: RunnerProfile {
                id: id.to_owned(),
                kind,
                output_formats: vec![RunnerOutputFormat::Binary, RunnerOutputFormat::PlainTextLog],
                backend_hint: Some("designer".to_owned()),
            },
            client_mode: Some(LaunchClientModeRequest::Designer),
            timeouts: ExecutionTimeouts::default(),
            policy: ExecutionPolicy {
                retain_artifacts_on_failure: true,
                retain_artifacts_on_success: true,
            },
            launch: LaunchOptions::default(),
        }
    }
}

/// Отказ синонима `designer-modules`, у которого режим обязателен: так вела себя
/// `/CheckModules`, и один цикл синоним ведёт себя ровно так же.
pub const MODULES_WITHOUT_MODES_ERROR: &str =
    "check designer-modules requires at least one mode flag";

/// Transport-neutral request for the `syntax` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxRequest {
    /// Selected syntax target and validation flags.
    pub target: SyntaxTargetRequest,
    /// Preview: locate the utility and name the plan, dispatch nothing.
    pub dry_run: bool,
}

/// Transport-neutral syntax target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxTargetRequest {
    DesignerConfig(DesignerConfigSyntaxRequest),
    /// Runs EDT validation for selected projects or all EDT projects when empty.
    Edt {
        projects: Vec<String>,
    },
}

/// Supported Designer client-mode scopes for syntax checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignerClientScope {
    ThinClient,
    WebClient,
    MobileClient,
    Server,
    ExternalConnection,
    ExternalConnectionServer,
    MobileAppClient,
    MobileAppServer,
    ThickClientManagedApplication,
    ThickClientServerManagedApplication,
    ThickClientOrdinaryApplication,
    ThickClientServerOrdinaryApplication,
}

impl DesignerClientScope {
    pub const fn flag(self) -> &'static str {
        match self {
            Self::ThinClient => "-ThinClient",
            Self::WebClient => "-WebClient",
            Self::MobileClient => "-MobileClient",
            Self::Server => "-Server",
            Self::ExternalConnection => "-ExternalConnection",
            Self::ExternalConnectionServer => "-ExternalConnectionServer",
            Self::MobileAppClient => "-MobileAppClient",
            Self::MobileAppServer => "-MobileAppServer",
            Self::ThickClientManagedApplication => "-ThickClientManagedApplication",
            Self::ThickClientServerManagedApplication => "-ThickClientServerManagedApplication",
            Self::ThickClientOrdinaryApplication => "-ThickClientOrdinaryApplication",
            Self::ThickClientServerOrdinaryApplication => "-ThickClientServerOrdinaryApplication",
        }
    }
}

/// Deduplicated client-scope set emitted in stable flag order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesignerClientScopes {
    scopes: Vec<DesignerClientScope>,
}

impl DesignerClientScopes {
    pub fn new(scopes: impl IntoIterator<Item = DesignerClientScope>) -> Self {
        let mut unique = Vec::new();
        for scope in scopes {
            if !unique.contains(&scope) {
                unique.push(scope);
            }
        }
        Self { scopes: unique }
    }

    pub fn contains(&self, scope: DesignerClientScope) -> bool {
        self.scopes.contains(&scope)
    }

    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

/// Supported non-client Designer configuration checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignerConfigCheck {
    ConfigLogIntegrity,
    IncorrectReferences,
    MobileClientDigiSign,
    DistributiveModules,
    UnreferenceProcedures,
    HandlersExistence,
    EmptyHandlers,
    UnsupportedFunctional,
}

impl DesignerConfigCheck {
    pub const fn flag(self) -> &'static str {
        match self {
            Self::ConfigLogIntegrity => "-ConfigLogIntegrity",
            Self::IncorrectReferences => "-IncorrectReferences",
            Self::MobileClientDigiSign => "-MobileClientDigiSign",
            Self::DistributiveModules => "-DistributiveModules",
            Self::UnreferenceProcedures => "-UnreferenceProcedures",
            Self::HandlersExistence => "-HandlersExistence",
            Self::EmptyHandlers => "-EmptyHandlers",
            Self::UnsupportedFunctional => "-UnsupportedFunctional",
        }
    }
}

/// Deduplicated config-check set emitted in stable flag order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesignerConfigChecks {
    checks: Vec<DesignerConfigCheck>,
}

impl DesignerConfigChecks {
    pub fn new(checks: impl IntoIterator<Item = DesignerConfigCheck>) -> Self {
        let mut unique = Vec::new();
        for check in checks {
            if !unique.contains(&check) {
                unique.push(check);
            }
        }
        Self { checks: unique }
    }

    pub fn contains(&self, check: DesignerConfigCheck) -> bool {
        self.checks.contains(&check)
    }

    pub fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }
}

/// Extension-targeting strategy for Designer syntax commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxExtensionScope {
    MainConfiguration,
    AllExtensions,
    SingleExtension { name: String },
    SingleExtensionAndAll { name: String },
}

impl SyntaxExtensionScope {
    pub fn new(extension: Option<String>, all_extensions: bool) -> Self {
        match (extension, all_extensions) {
            (Some(name), true) => Self::SingleExtensionAndAll { name },
            (Some(name), false) => Self::SingleExtension { name },
            (None, true) => Self::AllExtensions,
            (None, false) => Self::MainConfiguration,
        }
    }

    pub fn extension(&self) -> Option<&str> {
        match self {
            Self::SingleExtension { name } | Self::SingleExtensionAndAll { name } => {
                Some(name.as_str())
            }
            Self::MainConfiguration | Self::AllExtensions => None,
        }
    }

    pub const fn includes_all_extensions(&self) -> bool {
        matches!(
            self,
            Self::AllExtensions | Self::SingleExtensionAndAll { .. }
        )
    }
}

/// Fine-grained extra checks that can be enabled for extended modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedModulesDetail {
    Basic,
    SynchronousCalls,
    Modality,
    SynchronousCallsAndModality,
}

/// Typed policy for extended-modules validation instead of separate bool flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedModulesPolicy {
    Disabled,
    Enabled(ExtendedModulesDetail),
}

impl ExtendedModulesPolicy {
    pub fn from_cli_flags(
        enabled: bool,
        check_use_synchronous_calls: bool,
        check_use_modality: bool,
    ) -> Result<Self, UseCaseError> {
        Self::from_flags(
            enabled,
            check_use_synchronous_calls,
            check_use_modality,
            "check-use-synchronous-calls",
            "check-use-modality",
            "extended-modules-check",
        )
    }

    pub fn from_mcp_flags(
        enabled: Option<bool>,
        check_use_synchronous_calls: Option<bool>,
        check_use_modality: Option<bool>,
    ) -> Result<Self, UseCaseError> {
        Self::from_flags(
            enabled != Some(false),
            check_use_synchronous_calls == Some(true),
            check_use_modality == Some(true),
            "checkUseSynchronousCalls",
            "checkUseModality",
            "extendedModulesCheck",
        )
    }

    pub const fn basic(enabled: bool) -> Self {
        if enabled {
            Self::Enabled(ExtendedModulesDetail::Basic)
        } else {
            Self::Disabled
        }
    }

    fn from_flags(
        enabled: bool,
        check_use_synchronous_calls: bool,
        check_use_modality: bool,
        sync_calls_field: &'static str,
        modality_field: &'static str,
        enabled_field: &'static str,
    ) -> Result<Self, UseCaseError> {
        if !enabled && check_use_synchronous_calls {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("{sync_calls_field} requires {enabled_field}=true"),
            ));
        }

        if !enabled && check_use_modality {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("{modality_field} requires {enabled_field}=true"),
            ));
        }

        Ok(
            match (enabled, check_use_synchronous_calls, check_use_modality) {
                (false, false, false) => Self::Disabled,
                (true, false, false) => Self::Enabled(ExtendedModulesDetail::Basic),
                (true, true, false) => Self::Enabled(ExtendedModulesDetail::SynchronousCalls),
                (true, false, true) => Self::Enabled(ExtendedModulesDetail::Modality),
                (true, true, true) => {
                    Self::Enabled(ExtendedModulesDetail::SynchronousCallsAndModality)
                }
                (false, _, _) => unreachable!("dependency checks reject disabled extra checks"),
            },
        )
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    pub const fn checks_synchronous_calls(self) -> bool {
        matches!(
            self,
            Self::Enabled(ExtendedModulesDetail::SynchronousCalls)
                | Self::Enabled(ExtendedModulesDetail::SynchronousCallsAndModality)
        )
    }

    pub const fn checks_modality(self) -> bool {
        matches!(
            self,
            Self::Enabled(ExtendedModulesDetail::Modality)
                | Self::Enabled(ExtendedModulesDetail::SynchronousCallsAndModality)
        )
    }
}

/// Transport-neutral request for Designer configuration checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignerConfigSyntaxRequest {
    /// Non-client config checks selected for this request.
    checks: DesignerConfigChecks,
    /// Client scopes selected for Designer syntax analysis.
    client_scopes: DesignerClientScopes,
    /// Typed extended-modules policy, including dependent extra checks.
    extended_modules: ExtendedModulesPolicy,
    /// Extension-targeting strategy for the syntax run.
    extension_scope: SyntaxExtensionScope,
}

impl DesignerConfigSyntaxRequest {
    /// Режимы, которыми `check` проверяет конфигурацию, когда ни один не назван.
    ///
    /// Сайт обещает `/CheckConfig` со всеми режимами, а пустая `/CheckConfig` не проверяет
    /// ничего и отвечает «чисто» — это обещание наоборот. Профиль один на оба транспорта:
    /// MCP вёз его и раньше, командная строка теперь везёт тот же.
    pub fn default_profile(extension_scope: SyntaxExtensionScope) -> Self {
        Self {
            checks: DesignerConfigChecks::new([
                DesignerConfigCheck::UnreferenceProcedures,
                DesignerConfigCheck::HandlersExistence,
                DesignerConfigCheck::EmptyHandlers,
            ]),
            client_scopes: DesignerClientScopes::new([
                DesignerClientScope::ThinClient,
                DesignerClientScope::Server,
            ]),
            extended_modules: ExtendedModulesPolicy::basic(true),
            extension_scope,
        }
    }

    /// Назван ли хоть один режим: пустой набор означает профиль по умолчанию.
    pub fn names_no_mode(&self) -> bool {
        self.checks.is_empty()
            && self.client_scopes.is_empty()
            && !self.extended_modules.is_enabled()
    }

    pub fn new(
        checks: DesignerConfigChecks,
        client_scopes: DesignerClientScopes,
        extended_modules: ExtendedModulesPolicy,
        extension_scope: SyntaxExtensionScope,
    ) -> Self {
        Self {
            checks,
            client_scopes,
            extended_modules,
            extension_scope,
        }
    }

    pub fn has_check(&self, check: DesignerConfigCheck) -> bool {
        self.checks.contains(check)
    }

    pub fn has_client_scope(&self, scope: DesignerClientScope) -> bool {
        self.client_scopes.contains(scope)
    }

    pub const fn extended_modules(&self) -> ExtendedModulesPolicy {
        self.extended_modules
    }

    pub const fn extension_scope(&self) -> &SyntaxExtensionScope {
        &self.extension_scope
    }
}

/// Enterprise-mode launch targets grouped under the shared enterprise launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterpriseLaunchTarget {
    ThinClient,
    ThickClient,
    OrdinaryApplication,
    ClientMcp { mode: ClientMcpMode },
}

/// Client mode used by the 1C client-side MCP launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMcpMode {
    Thin,
    Thick,
    Ordinary,
}

/// Transport-neutral launch target grouped by launcher family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTargetRequest {
    Designer,
    Enterprise(EnterpriseLaunchTarget),
    /// The published infobase in the browser at `infobase.web.url`.
    Web,
}

impl LaunchTargetRequest {
    pub const fn designer() -> Self {
        Self::Designer
    }

    pub const fn web() -> Self {
        Self::Web
    }

    pub const fn thin_client() -> Self {
        Self::Enterprise(EnterpriseLaunchTarget::ThinClient)
    }

    pub const fn thick_client() -> Self {
        Self::Enterprise(EnterpriseLaunchTarget::ThickClient)
    }

    pub const fn ordinary_application() -> Self {
        Self::Enterprise(EnterpriseLaunchTarget::OrdinaryApplication)
    }

    pub const fn client_mcp() -> Self {
        Self::Enterprise(EnterpriseLaunchTarget::ClientMcp {
            mode: ClientMcpMode::Thin,
        })
    }

    pub const fn client_mcp_with_mode(mode: ClientMcpMode) -> Self {
        Self::Enterprise(EnterpriseLaunchTarget::ClientMcp { mode })
    }
}

/// Optional scenario launched alongside the client-side MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMcpAddonRequest {
    VanessaAutomation,
}

/// Transport-neutral options for `launch mcp`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientMcpOptionsRequest {
    pub config_path: Option<String>,
    pub port: Option<u16>,
    pub addon: Option<ClientMcpAddonRequest>,
    pub wait_ready: bool,
}

/// Transport-neutral request for the `launch` use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Requested launch target.
    pub target: LaunchTargetRequest,
    /// Shared launch options mapped from CLI/test scenarios.
    pub launch: LaunchOptions,
    /// Client-side MCP launch options. Present only for `LaunchTargetRequest::client_mcp*`.
    pub client_mcp: Option<ClientMcpOptionsRequest>,
    /// Каким адресом открыть базу. `None` — умолчание по виду цели.
    pub via: Option<LaunchVia>,
    /// Validate and select a provider without launching the client process.
    pub dry_run: bool,
}

/// Transport-neutral request for the `init` use case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitRequest {
    /// Decide every step and locate the platform without creating anything.
    pub dry_run: bool,
}

/// Transport-neutral request for extension property updates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigureExtensionsRequest {
    /// Explicit source-set names. Both selector lists empty means all extension source-sets.
    pub names: Vec<String>,
    /// Explicit installed platform names, independent of configured source-sets.
    pub installed_names: Vec<String>,
    /// Resolve targets and locate ibcmd without starting it or changing the infobase.
    pub dry_run: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        DesignerClientScope, DesignerClientScopes, DesignerConfigCheck, DesignerConfigChecks,
        DesignerConfigSyntaxRequest, ExtendedModulesDetail, ExtendedModulesPolicy,
        SyntaxExtensionScope,
    };
    use crate::use_cases::result::UseCaseErrorKind;

    #[test]
    fn config_request_uses_typed_policy_objects() {
        let request = DesignerConfigSyntaxRequest::new(
            DesignerConfigChecks::new([
                DesignerConfigCheck::ConfigLogIntegrity,
                DesignerConfigCheck::ConfigLogIntegrity,
                DesignerConfigCheck::UnsupportedFunctional,
            ]),
            DesignerClientScopes::new([
                DesignerClientScope::ThinClient,
                DesignerClientScope::ThinClient,
                DesignerClientScope::Server,
            ]),
            ExtendedModulesPolicy::from_cli_flags(true, true, false).expect("policy"),
            SyntaxExtensionScope::new(Some("Ext".to_owned()), true),
        );

        assert!(request.has_check(DesignerConfigCheck::ConfigLogIntegrity));
        assert!(request.has_check(DesignerConfigCheck::UnsupportedFunctional));
        assert!(request.has_client_scope(DesignerClientScope::ThinClient));
        assert!(request.has_client_scope(DesignerClientScope::Server));
        assert_eq!(
            request.extended_modules(),
            ExtendedModulesPolicy::Enabled(ExtendedModulesDetail::SynchronousCalls)
        );
        assert_eq!(request.extension_scope().extension(), Some("Ext"));
        assert!(request.extension_scope().includes_all_extensions());
    }

    #[test]
    fn extended_modules_policy_rejects_invalid_dependency_combinations() {
        let error =
            ExtendedModulesPolicy::from_mcp_flags(Some(false), Some(true), None).expect_err("err");

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(
            error.message(),
            "checkUseSynchronousCalls requires extendedModulesCheck=true"
        );
    }

    /// Требование «хотя бы один режим» жило у `/CheckModules`; у `/CheckConfig` его нет,
    /// а пустой запрос выполняет профиль по умолчанию.
    #[test]
    fn a_config_request_without_modes_names_no_mode() {
        let request = DesignerConfigSyntaxRequest::new(
            DesignerConfigChecks::default(),
            DesignerClientScopes::default(),
            ExtendedModulesPolicy::basic(false),
            SyntaxExtensionScope::new(None, true),
        );

        assert!(request.names_no_mode());
        assert!(
            !DesignerConfigSyntaxRequest::default_profile(SyntaxExtensionScope::new(None, true))
                .names_no_mode()
        );
    }
}

/// Which part of the infobase extension composition to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionInventoryScope {
    /// Every extension installed in the infobase.
    All,
    /// One extension by name; a reply without it is refused as an invalid result.
    Named { name: String },
}

/// Request to read the extension composition of the configured infobase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionInventoryRequest {
    pub scope: ExtensionInventoryScope,
    /// Locate the utility and name the target without starting the platform.
    pub dry_run: bool,
}
