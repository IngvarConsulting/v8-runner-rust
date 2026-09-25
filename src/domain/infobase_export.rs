use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::execution::{
    ExecutionInterruptionPhase, ExecutionOutcome, ExecutionStatus, ExecutionStepKind, StepResult,
};

/// Closed vocabulary for every observable phase of an information-base transfer — both
/// directions — including failures before provider dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfobaseTransferPhase {
    ConfigurationLoad,
    Validation,
    ProviderSelection,
    WorkspaceLock,
    WorkspacePreparation,
    ResolveTarget,
    TargetLock,
    OrphanCleanup,
    PrepareStaging,
    ProviderCommand,
    ValidateProviderOutput,
    BeforePublication,
    PublishTargetRevalidation,
    Publication,
}

impl InfobaseTransferPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigurationLoad => "configuration load",
            Self::Validation => "validation",
            Self::ProviderSelection => "provider selection",
            Self::WorkspaceLock => "workspace lock",
            Self::WorkspacePreparation => "workspace preparation",
            Self::ResolveTarget => "resolve target",
            Self::TargetLock => "target lock",
            Self::OrphanCleanup => "orphan cleanup",
            Self::PrepareStaging => "prepare staging",
            Self::ProviderCommand => "provider command",
            Self::ValidateProviderOutput => "validate provider output",
            Self::BeforePublication => "before publication",
            Self::PublishTargetRevalidation => "publish target revalidation",
            Self::Publication => "publication",
        }
    }

    /// Что было прервано на этой фазе переноса. Шаг называет `steps[]`; фаза прерывания —
    /// только прерванную работу: процесс исполнителя, публикацию или границу команды.
    pub const fn interruption_phase(self) -> ExecutionInterruptionPhase {
        match self {
            Self::ProviderCommand => ExecutionInterruptionPhase::ProviderCommand,
            Self::Publication => ExecutionInterruptionPhase::Publication,
            Self::ConfigurationLoad
            | Self::Validation
            | Self::ProviderSelection
            | Self::WorkspaceLock
            | Self::WorkspacePreparation
            | Self::ResolveTarget
            | Self::TargetLock
            | Self::OrphanCleanup
            | Self::PrepareStaging
            | Self::ValidateProviderOutput
            | Self::BeforePublication
            | Self::PublishTargetRevalidation => ExecutionInterruptionPhase::CommandBoundary,
        }
    }

    pub const fn kind(self) -> ExecutionStepKind {
        match self {
            Self::ConfigurationLoad | Self::Validation | Self::ValidateProviderOutput => {
                ExecutionStepKind::Validation
            }
            Self::WorkspaceLock
            | Self::WorkspacePreparation
            | Self::OrphanCleanup
            | Self::PrepareStaging => ExecutionStepKind::PrepareWorkspace,
            Self::ResolveTarget | Self::TargetLock | Self::PublishTargetRevalidation => {
                ExecutionStepKind::ResolveTarget
            }
            Self::ProviderSelection => ExecutionStepKind::Other,
            Self::ProviderCommand => ExecutionStepKind::PlatformCommand,
            Self::BeforePublication | Self::Publication => ExecutionStepKind::Publish,
        }
    }
}

/// Configuration state persisted into a package.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationState {
    Working,
    Database,
}

impl ConfigurationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Database => "database",
        }
    }
}

/// Configuration whose state is persisted into a package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigurationSubject {
    Main,
    Extension { name: String },
}

impl ConfigurationSubject {
    pub const fn artifact_kind(&self) -> InfobaseExportArtifactKind {
        match self {
            Self::Main => InfobaseExportArtifactKind::Cf,
            Self::Extension { .. } => InfobaseExportArtifactKind::Cfe,
        }
    }
}

use crate::domain::capability::{Provider, ProviderReceipt};

/// Closed file format vocabulary for information-base exports.
///
/// This type is deliberately namespaced: [`crate::domain::artifact::ArtifactKind`]
/// classifies retained execution artifacts and does not distinguish CF, CFE,
/// and DT package formats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InfobaseExportArtifactKind {
    Cf,
    Cfe,
    Dt,
}

impl InfobaseExportArtifactKind {
    pub const fn file_extension(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Cfe => "cfe",
            Self::Dt => "dt",
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.file_extension()
    }
}

/// Subject marker for a complete DT export.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InfobaseSnapshotSubject {
    Infobase,
}

/// Observable state of the final output path after an export attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportTargetState {
    Unchanged,
    Created,
    Replaced,
    Restored,
    Uncertain,
}

/// Whether the command only proves its execution plan or applies it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InfobaseExportMode {
    Preview,
    Apply,
}

/// Compact machine-facing plan produced by a non-executing preflight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InfobaseExportPlan {
    pub provider: Provider,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
}

/// Request to persist a working or database configuration into CF/CFE.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportConfigurationPackageRequest {
    pub state: ConfigurationState,
    pub subject: ConfigurationSubject,
    pub output: PathBuf,
}

/// Typed presentation data for a configuration package export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportConfigurationPackageResult {
    pub mode: InfobaseExportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_dispatched: Option<bool>,
    pub state: ConfigurationState,
    pub subject: ConfigurationSubject,
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
    pub published: bool,
    pub target_state: ExportTargetState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<InfobaseExportPlan>,
    #[serde(skip)]
    pub warnings: Vec<String>,
    pub execution: ExecutionOutcome<()>,
    #[serde(skip)]
    pub steps: Vec<StepResult>,
}

impl ExportConfigurationPackageResult {
    pub fn new(
        request: ExportConfigurationPackageRequest,
        provider: Option<ProviderReceipt>,
    ) -> Self {
        let artifact_kind = request.subject.artifact_kind();
        Self {
            mode: InfobaseExportMode::Apply,
            provider_dispatched: None,
            state: request.state,
            subject: request.subject,
            provider,
            artifact_kind,
            output: request.output,
            published: false,
            target_state: ExportTargetState::Unchanged,
            plan: None,
            warnings: Vec::new(),
            execution: ExecutionOutcome::new(ExecutionStatus::Failed),
            steps: Vec::new(),
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
        self.plan = self
            .provider
            .as_ref()
            .and_then(|receipt| receipt.selected)
            .map(|provider| InfobaseExportPlan {
                provider,
                artifact_kind: self.artifact_kind,
                output: self.output.clone(),
            });
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview_failure(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
    }
}

/// Request to persist the complete information base into a DT snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportInfobaseSnapshotRequest {
    pub output: PathBuf,
}

/// Typed presentation data for an information-base snapshot export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExportInfobaseSnapshotResult {
    pub mode: InfobaseExportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_dispatched: Option<bool>,
    pub subject: InfobaseSnapshotSubject,
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
    pub published: bool,
    pub target_state: ExportTargetState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<InfobaseExportPlan>,
    #[serde(skip)]
    pub warnings: Vec<String>,
    pub execution: ExecutionOutcome<()>,
    #[serde(skip)]
    pub steps: Vec<StepResult>,
}

impl ExportInfobaseSnapshotResult {
    pub fn new(request: ExportInfobaseSnapshotRequest, provider: Option<ProviderReceipt>) -> Self {
        Self {
            mode: InfobaseExportMode::Apply,
            provider_dispatched: None,
            subject: InfobaseSnapshotSubject::Infobase,
            provider,
            artifact_kind: InfobaseExportArtifactKind::Dt,
            output: request.output,
            published: false,
            target_state: ExportTargetState::Unchanged,
            plan: None,
            warnings: Vec::new(),
            execution: ExecutionOutcome::new(ExecutionStatus::Failed),
            steps: Vec::new(),
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
        self.plan = self
            .provider
            .as_ref()
            .and_then(|receipt| receipt.selected)
            .map(|provider| InfobaseExportPlan {
                provider,
                artifact_kind: self.artifact_kind,
                output: self.output.clone(),
            });
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview_failure(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
    }
}

/// Which irreversible change to the target infobase the caller permits.
///
/// Neither provider asks: Designer creates an absent infobase and overwrites a
/// present one, and IBCMD overwrites a present one. The mode is therefore a
/// runner-side gate, and a mode that does not match the observed target is a
/// refusal rather than a silent fallback to the other case.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreTargetMode {
    /// The target infobase must be absent; restoring creates it.
    Create,
    /// The target infobase must exist; restoring discards its current data.
    Replace,
}

impl RestoreTargetMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
        }
    }
}

/// Request to load a complete information base from a DT transfer file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct RestoreInfobaseSnapshotRequest {
    pub input: PathBuf,
    pub target_mode: RestoreTargetMode,
}

/// Compact machine-facing plan produced by a non-executing restore preflight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InfobaseRestorePlan {
    pub provider: Provider,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub input: PathBuf,
    pub target_mode: RestoreTargetMode,
}

/// Typed presentation data for an information-base restore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct RestoreInfobaseSnapshotResult {
    pub mode: InfobaseExportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_dispatched: Option<bool>,
    pub subject: InfobaseSnapshotSubject,
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub input: PathBuf,
    pub target_mode: RestoreTargetMode,
    /// `true` only after the provider reported a completed load.
    pub restored: bool,
    pub target_state: ExportTargetState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<InfobaseRestorePlan>,
    #[serde(skip)]
    pub warnings: Vec<String>,
    pub execution: ExecutionOutcome<()>,
    #[serde(skip)]
    pub steps: Vec<StepResult>,
}

impl RestoreInfobaseSnapshotResult {
    pub fn new(request: RestoreInfobaseSnapshotRequest, provider: Option<ProviderReceipt>) -> Self {
        Self {
            mode: InfobaseExportMode::Apply,
            provider_dispatched: None,
            subject: InfobaseSnapshotSubject::Infobase,
            provider,
            artifact_kind: InfobaseExportArtifactKind::Dt,
            input: request.input,
            target_mode: request.target_mode,
            restored: false,
            target_state: ExportTargetState::Unchanged,
            plan: None,
            warnings: Vec::new(),
            execution: ExecutionOutcome::new(ExecutionStatus::Failed),
            steps: Vec::new(),
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
        self.plan = self
            .provider
            .as_ref()
            .and_then(|receipt| receipt.selected)
            .map(|provider| InfobaseRestorePlan {
                provider,
                artifact_kind: self.artifact_kind,
                input: self.input.clone(),
                target_mode: self.target_mode,
            });
        self.execution.status = ExecutionStatus::Succeeded;
    }

    pub fn mark_preview_failure(&mut self) {
        self.mode = InfobaseExportMode::Preview;
        self.provider_dispatched = Some(false);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        ConfigurationState, ConfigurationSubject, ExportConfigurationPackageRequest,
        ExportConfigurationPackageResult, ExportInfobaseSnapshotRequest,
        ExportInfobaseSnapshotResult, ExportTargetState, InfobaseExportArtifactKind,
        InfobaseTransferPhase, Provider, ProviderReceipt,
    };
    use crate::domain::capability::Implementation;
    use crate::domain::execution::ExecutionStepKind;

    fn chosen(provider: Provider) -> Option<ProviderReceipt> {
        Some(ProviderReceipt::new(
            provider,
            crate::domain::capability::ProviderOrigin::Default,
        ))
    }

    #[test]
    fn transfer_phase_is_the_single_owner_of_step_name_and_kind() {
        assert_eq!(
            InfobaseTransferPhase::ConfigurationLoad.as_str(),
            "configuration load"
        );
        assert_eq!(
            InfobaseTransferPhase::ConfigurationLoad.kind(),
            ExecutionStepKind::Validation
        );
        assert_eq!(
            InfobaseTransferPhase::WorkspaceLock.as_str(),
            "workspace lock"
        );
        assert_eq!(
            InfobaseTransferPhase::WorkspaceLock.kind(),
            ExecutionStepKind::PrepareWorkspace
        );
        assert_eq!(
            InfobaseTransferPhase::WorkspacePreparation.as_str(),
            "workspace preparation"
        );
        assert_eq!(
            InfobaseTransferPhase::WorkspacePreparation.kind(),
            ExecutionStepKind::PrepareWorkspace
        );
        assert_eq!(
            InfobaseTransferPhase::ProviderCommand.kind(),
            ExecutionStepKind::PlatformCommand
        );
    }

    #[test]
    fn configuration_request_serializes_closed_transport_neutral_vocabulary() {
        let request = ExportConfigurationPackageRequest {
            state: ConfigurationState::Database,
            subject: ConfigurationSubject::Extension {
                name: "Sales".to_owned(),
            },
            output: PathBuf::from("/tmp/sales.cfe"),
        };

        assert_eq!(
            serde_json::to_value(&request).expect("configuration request json"),
            json!({
                "state": "database",
                "subject": {"kind": "extension", "name": "Sales"},
                "output": "/tmp/sales.cfe"
            })
        );
        assert_eq!(
            serde_json::to_value(Provider::Designer).expect("provider json"),
            json!("designer")
        );
        assert_eq!(
            serde_json::to_value(Implementation::Experimental).expect("implementation json"),
            json!("experimental")
        );
        assert_eq!(
            request.subject.artifact_kind(),
            InfobaseExportArtifactKind::Cfe
        );
    }

    #[test]
    fn configuration_result_derives_artifact_kind_from_subject() {
        let request = ExportConfigurationPackageRequest {
            state: ConfigurationState::Working,
            subject: ConfigurationSubject::Main,
            output: PathBuf::from("/tmp/main.cf"),
        };
        let result = ExportConfigurationPackageResult::new(request, chosen(Provider::Ibcmd));

        assert_eq!(result.artifact_kind, InfobaseExportArtifactKind::Cf);
        assert!(!result.published);
        assert!(result.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(result).expect("configuration result json"),
            json!({
                "mode": "apply",
                "state": "working",
                "subject": {"kind": "main"},
                "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
                "artifact_kind": "cf",
                "output": "/tmp/main.cf",
                "published": false,
                "target_state": "unchanged",
                "execution": {"status": "failed"}
            })
        );
    }

    #[test]
    fn snapshot_result_is_always_a_dt_and_preserves_typed_presentation_fields() {
        let request = ExportInfobaseSnapshotRequest {
            output: PathBuf::from("/tmp/base.dt"),
        };
        let mut result = ExportInfobaseSnapshotResult::new(request, chosen(Provider::Designer));
        result.published = true;
        result.target_state = ExportTargetState::Created;
        result.mark_succeeded();
        result.warnings.push("staging cleanup deferred".to_owned());

        assert_eq!(result.artifact_kind, InfobaseExportArtifactKind::Dt);
        assert_eq!(
            serde_json::to_value(result).expect("snapshot result json"),
            json!({
                "mode": "apply",
                "subject": {"kind": "infobase"},
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "artifact_kind": "dt",
                "output": "/tmp/base.dt",
                "published": true,
                "target_state": "created",
                "execution": {"status": "succeeded"}
            })
        );
    }
}
