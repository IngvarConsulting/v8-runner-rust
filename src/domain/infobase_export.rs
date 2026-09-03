use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::execution::{ExecutionOutcome, ExecutionStatus, ExecutionStepKind, StepResult};

/// Closed vocabulary for every observable phase of an information-base export,
/// including failures before provider dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
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

impl ExportPhase {
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Whether runner has an adapter for an exact export operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderImplementation {
    Implemented,
    Experimental,
    Unsupported,
}

impl ProviderImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Current-environment readiness established without starting a provider process.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    Ready,
    Unavailable,
    NotChecked,
}

impl ProviderReadiness {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unavailable => "unavailable",
            Self::NotChecked => "not_checked",
        }
    }
}

/// Strongest evidence currently attached to an implementation row.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEvidence {
    Documented,
    ArgvTested,
    LiveVerified,
}

impl ProviderEvidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Documented => "documented",
            Self::ArgvTested => "argv_tested",
            Self::LiveVerified => "live_verified",
        }
    }
}

/// Process provider selected for an information-base export.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExportProvider {
    DesignerBatch,
    IbcmdProcess,
}

impl ExportProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesignerBatch => "designer-batch",
            Self::IbcmdProcess => "ibcmd-process",
        }
    }
}

/// Closed file format vocabulary for information-base exports.
///
/// This type is deliberately namespaced: [`crate::domain::artifact::ArtifactKind`]
/// classifies retained execution artifacts and does not distinguish CF, CFE,
/// and DT package formats.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InfobaseSnapshotSubject {
    Infobase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCandidate {
    pub provider: ExportProvider,
    pub implementation: ProviderImplementation,
    pub readiness: ProviderReadiness,
    pub evidence: ProviderEvidence,
    pub reason: String,
}

impl ProviderCandidate {
    pub fn new(
        provider: ExportProvider,
        implementation: ProviderImplementation,
        readiness: ProviderReadiness,
        evidence: ProviderEvidence,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            implementation,
            readiness,
            evidence,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportProviderDecision {
    provider: Option<ExportProvider>,
    reason: String,
    candidates: Vec<ProviderCandidate>,
}

impl ExportProviderDecision {
    pub fn selected(
        provider: ExportProvider,
        reason: impl Into<String>,
        candidates: Vec<ProviderCandidate>,
    ) -> Self {
        debug_assert!(candidates.iter().any(|candidate| {
            candidate.provider == provider
                && candidate.implementation == ProviderImplementation::Implemented
                && candidate.readiness == ProviderReadiness::Ready
        }));
        Self {
            provider: Some(provider),
            reason: reason.into(),
            candidates,
        }
    }

    pub fn unavailable(reason: impl Into<String>, candidates: Vec<ProviderCandidate>) -> Self {
        Self {
            provider: None,
            reason: reason.into(),
            candidates,
        }
    }

    pub const fn provider(&self) -> Option<ExportProvider> {
        self.provider
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn candidates(&self) -> &[ProviderCandidate] {
        &self.candidates
    }
}

/// Observable state of the final output path after an export attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportTargetState {
    Unchanged,
    Created,
    Replaced,
    Restored,
    Uncertain,
}

/// Whether the command only proves its execution plan or applies it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InfobaseExportMode {
    Preview,
    Apply,
}

/// Compact machine-facing plan produced by a non-executing preflight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfobaseExportPlan {
    pub provider: ExportProvider,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
}

/// Request to persist a working or database configuration into CF/CFE.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportConfigurationPackageRequest {
    pub state: ConfigurationState,
    pub subject: ConfigurationSubject,
    pub output: PathBuf,
}

/// Typed presentation data for a configuration package export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportConfigurationPackageResult {
    pub mode: InfobaseExportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_dispatched: Option<bool>,
    pub state: ConfigurationState,
    pub subject: ConfigurationSubject,
    pub selection: ExportProviderDecision,
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
        selection: ExportProviderDecision,
    ) -> Self {
        let artifact_kind = request.subject.artifact_kind();
        Self {
            mode: InfobaseExportMode::Apply,
            provider_dispatched: None,
            state: request.state,
            subject: request.subject,
            selection,
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
            .selection
            .provider()
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportInfobaseSnapshotRequest {
    pub output: PathBuf,
}

/// Typed presentation data for an information-base snapshot export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportInfobaseSnapshotResult {
    pub mode: InfobaseExportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_dispatched: Option<bool>,
    pub subject: InfobaseSnapshotSubject,
    pub selection: ExportProviderDecision,
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
    pub fn new(request: ExportInfobaseSnapshotRequest, selection: ExportProviderDecision) -> Self {
        Self {
            mode: InfobaseExportMode::Apply,
            provider_dispatched: None,
            subject: InfobaseSnapshotSubject::Infobase,
            selection,
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
            .selection
            .provider()
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        ConfigurationState, ConfigurationSubject, ExportConfigurationPackageRequest,
        ExportConfigurationPackageResult, ExportInfobaseSnapshotRequest,
        ExportInfobaseSnapshotResult, ExportPhase, ExportProvider, ExportProviderDecision,
        ExportTargetState, InfobaseExportArtifactKind, ProviderCandidate, ProviderEvidence,
        ProviderImplementation, ProviderReadiness,
    };
    use crate::domain::execution::ExecutionStepKind;

    fn ready(provider: ExportProvider) -> ProviderCandidate {
        ProviderCandidate::new(
            provider,
            ProviderImplementation::Implemented,
            ProviderReadiness::Ready,
            ProviderEvidence::ArgvTested,
            "adapter and utility are ready",
        )
    }

    #[test]
    fn export_phase_is_the_single_owner_of_step_name_and_kind() {
        assert_eq!(
            ExportPhase::ConfigurationLoad.as_str(),
            "configuration load"
        );
        assert_eq!(
            ExportPhase::ConfigurationLoad.kind(),
            ExecutionStepKind::Validation
        );
        assert_eq!(ExportPhase::WorkspaceLock.as_str(), "workspace lock");
        assert_eq!(
            ExportPhase::WorkspaceLock.kind(),
            ExecutionStepKind::PrepareWorkspace
        );
        assert_eq!(
            ExportPhase::WorkspacePreparation.as_str(),
            "workspace preparation"
        );
        assert_eq!(
            ExportPhase::WorkspacePreparation.kind(),
            ExecutionStepKind::PrepareWorkspace
        );
        assert_eq!(
            ExportPhase::ProviderCommand.kind(),
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
            serde_json::to_value(ExportProvider::DesignerBatch).expect("provider json"),
            json!("designer-batch")
        );
        assert_eq!(
            serde_json::to_value(ProviderImplementation::Experimental)
                .expect("implementation json"),
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
        let selection = ExportProviderDecision::selected(
            ExportProvider::IbcmdProcess,
            "selected ready provider",
            vec![ready(ExportProvider::IbcmdProcess)],
        );

        let result = ExportConfigurationPackageResult::new(request, selection);

        assert_eq!(result.artifact_kind, InfobaseExportArtifactKind::Cf);
        assert!(!result.published);
        assert!(result.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(result).expect("configuration result json"),
            json!({
                "mode": "apply",
                "state": "working",
                "subject": {"kind": "main"},
                "selection": {
                    "provider": "ibcmd-process",
                    "reason": "selected ready provider",
                    "candidates": [{
                        "provider": "ibcmd-process",
                        "implementation": "implemented",
                        "readiness": "ready",
                        "evidence": "argv_tested",
                        "reason": "adapter and utility are ready"
                    }]
                },
                "artifact_kind": "cf",
                "output": "/tmp/main.cf",
                "published": false,
                "target_state": "unchanged",
                "execution": {"status": "failed"}
            })
        );
    }

    #[test]
    fn unavailable_provider_decision_keeps_candidate_diagnostics() {
        let decision = ExportProviderDecision::unavailable(
            "no implemented provider is ready for DT export",
            vec![ProviderCandidate::new(
                ExportProvider::IbcmdProcess,
                ProviderImplementation::Experimental,
                ProviderReadiness::NotChecked,
                ProviderEvidence::Documented,
                "exclusive access is not implemented",
            )],
        );

        assert_eq!(decision.provider(), None);
        assert_eq!(
            serde_json::to_value(&decision).expect("unsupported decision json"),
            json!({
                "provider": null,
                "reason": "no implemented provider is ready for DT export",
                "candidates": [{
                    "provider": "ibcmd-process",
                    "implementation": "experimental",
                    "readiness": "not_checked",
                    "evidence": "documented",
                    "reason": "exclusive access is not implemented"
                }]
            })
        );
    }

    #[test]
    fn snapshot_result_is_always_a_dt_and_preserves_typed_presentation_fields() {
        let request = ExportInfobaseSnapshotRequest {
            output: PathBuf::from("/tmp/base.dt"),
        };
        let selection = ExportProviderDecision::selected(
            ExportProvider::DesignerBatch,
            "selected ready provider",
            vec![ready(ExportProvider::DesignerBatch)],
        );
        let mut result = ExportInfobaseSnapshotResult::new(request, selection);
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
                "selection": {
                    "provider": "designer-batch",
                    "reason": "selected ready provider",
                    "candidates": [{
                        "provider": "designer-batch",
                        "implementation": "implemented",
                        "readiness": "ready",
                        "evidence": "argv_tested",
                        "reason": "adapter and utility are ready"
                    }]
                },
                "artifact_kind": "dt",
                "output": "/tmp/base.dt",
                "published": true,
                "target_state": "created",
                "execution": {"status": "succeeded"}
            })
        );
    }
}
