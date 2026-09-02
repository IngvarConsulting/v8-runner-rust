use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::execution::{ExecutionOutcome, ExecutionStatus, StepResult};

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

/// Strength of the evidence supporting a provider capability.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidence {
    Available,
    Unverified,
    Unsupported,
}

impl CapabilityEvidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unverified => "unverified",
            Self::Unsupported => "unsupported",
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
#[serde(try_from = "RawExportProviderDecision")]
pub struct ExportProviderDecision {
    provider: Option<ExportProvider>,
    evidence: CapabilityEvidence,
    provider_reason: String,
}

#[derive(Debug, Deserialize)]
struct RawExportProviderDecision {
    provider: Option<ExportProvider>,
    evidence: CapabilityEvidence,
    provider_reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProviderDecisionError {
    AvailableRequiresProvider,
    UnavailableCannotSelectProvider,
}

impl ExportProviderDecision {
    pub fn available(provider: ExportProvider, reason: impl Into<String>) -> Self {
        Self {
            provider: Some(provider),
            evidence: CapabilityEvidence::Available,
            provider_reason: reason.into(),
        }
    }

    pub fn unavailable(
        evidence: CapabilityEvidence,
        reason: impl Into<String>,
    ) -> Result<Self, ExportProviderDecisionError> {
        if evidence == CapabilityEvidence::Available {
            return Err(ExportProviderDecisionError::AvailableRequiresProvider);
        }

        Ok(Self {
            provider: None,
            evidence,
            provider_reason: reason.into(),
        })
    }

    pub const fn provider(&self) -> Option<ExportProvider> {
        self.provider
    }

    pub const fn evidence(&self) -> CapabilityEvidence {
        self.evidence
    }

    pub fn reason(&self) -> &str {
        &self.provider_reason
    }
}

impl TryFrom<RawExportProviderDecision> for ExportProviderDecision {
    type Error = ExportProviderDecisionError;

    fn try_from(value: RawExportProviderDecision) -> Result<Self, Self::Error> {
        match (value.provider, value.evidence) {
            (Some(provider), CapabilityEvidence::Available) => {
                Ok(Self::available(provider, value.provider_reason))
            }
            (None, CapabilityEvidence::Unverified) => {
                Self::unavailable(CapabilityEvidence::Unverified, value.provider_reason)
            }
            (None, CapabilityEvidence::Unsupported) => {
                Self::unavailable(CapabilityEvidence::Unsupported, value.provider_reason)
            }
            (None, CapabilityEvidence::Available) => {
                Err(ExportProviderDecisionError::AvailableRequiresProvider)
            }
            (Some(_), _) => Err(ExportProviderDecisionError::UnavailableCannotSelectProvider),
        }
    }
}

impl std::fmt::Display for ExportProviderDecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AvailableRequiresProvider => {
                formatter.write_str("available capability requires a selected provider")
            }
            Self::UnavailableCannotSelectProvider => {
                formatter.write_str("unverified or unsupported capability cannot select a provider")
            }
        }
    }
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
    pub state: ConfigurationState,
    pub subject: ConfigurationSubject,
    #[serde(flatten)]
    pub selection: ExportProviderDecision,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
    pub published: bool,
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
            state: request.state,
            subject: request.subject,
            selection,
            artifact_kind,
            output: request.output,
            published: false,
            warnings: Vec::new(),
            execution: ExecutionOutcome::new(ExecutionStatus::Failed),
            steps: Vec::new(),
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.execution.status = ExecutionStatus::Succeeded;
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
    pub subject: InfobaseSnapshotSubject,
    #[serde(flatten)]
    pub selection: ExportProviderDecision,
    pub artifact_kind: InfobaseExportArtifactKind,
    pub output: PathBuf,
    pub published: bool,
    #[serde(skip)]
    pub warnings: Vec<String>,
    pub execution: ExecutionOutcome<()>,
    #[serde(skip)]
    pub steps: Vec<StepResult>,
}

impl ExportInfobaseSnapshotResult {
    pub fn new(request: ExportInfobaseSnapshotRequest, selection: ExportProviderDecision) -> Self {
        Self {
            subject: InfobaseSnapshotSubject::Infobase,
            selection,
            artifact_kind: InfobaseExportArtifactKind::Dt,
            output: request.output,
            published: false,
            warnings: Vec::new(),
            execution: ExecutionOutcome::new(ExecutionStatus::Failed),
            steps: Vec::new(),
        }
    }

    pub fn mark_succeeded(&mut self) {
        self.execution.status = ExecutionStatus::Succeeded;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        CapabilityEvidence, ConfigurationState, ConfigurationSubject,
        ExportConfigurationPackageRequest, ExportConfigurationPackageResult,
        ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult, ExportProvider,
        ExportProviderDecision, InfobaseExportArtifactKind,
    };

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
            serde_json::to_value(CapabilityEvidence::Unverified).expect("evidence json"),
            json!("unverified")
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
        let selection = ExportProviderDecision::available(
            ExportProvider::IbcmdProcess,
            "configured builder has verified config-save support",
        );

        let result = ExportConfigurationPackageResult::new(request, selection);

        assert_eq!(result.artifact_kind, InfobaseExportArtifactKind::Cf);
        assert!(!result.published);
        assert!(result.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(result).expect("configuration result json"),
            json!({
                "state": "working",
                "subject": {"kind": "main"},
                "provider": "ibcmd-process",
                "evidence": "available",
                "provider_reason": "configured builder has verified config-save support",
                "artifact_kind": "cf",
                "output": "/tmp/main.cf",
                "published": false,
                "execution": {"status": "failed"}
            })
        );
    }

    #[test]
    fn unavailable_provider_decision_cannot_claim_a_selected_provider() {
        let decision = ExportProviderDecision::unavailable(
            CapabilityEvidence::Unsupported,
            "selected backend does not implement DT export",
        )
        .expect("unsupported decision");

        assert_eq!(decision.provider(), None);
        assert_eq!(decision.evidence(), CapabilityEvidence::Unsupported);
        assert_eq!(
            serde_json::to_value(&decision).expect("unsupported decision json"),
            json!({
                "provider": null,
                "evidence": "unsupported",
                "provider_reason": "selected backend does not implement DT export"
            })
        );
        assert!(ExportProviderDecision::unavailable(
            CapabilityEvidence::Available,
            "available requires a provider",
        )
        .is_err());
        assert!(serde_json::from_value::<ExportProviderDecision>(json!({
            "provider": "designer-batch",
            "evidence": "unverified",
            "provider_reason": "help text only"
        }))
        .is_err());
        assert!(serde_json::from_value::<ExportProviderDecision>(json!({
            "provider": null,
            "evidence": "available",
            "provider_reason": "missing selected provider"
        }))
        .is_err());
    }

    #[test]
    fn snapshot_result_is_always_a_dt_and_preserves_typed_presentation_fields() {
        let request = ExportInfobaseSnapshotRequest {
            output: PathBuf::from("/tmp/base.dt"),
        };
        let selection = ExportProviderDecision::available(
            ExportProvider::DesignerBatch,
            "designer batch DumpIB capability is verified",
        );
        let mut result = ExportInfobaseSnapshotResult::new(request, selection);
        result.published = true;
        result.mark_succeeded();
        result.warnings.push("staging cleanup deferred".to_owned());

        assert_eq!(result.artifact_kind, InfobaseExportArtifactKind::Dt);
        assert_eq!(
            serde_json::to_value(result).expect("snapshot result json"),
            json!({
                "subject": {"kind": "infobase"},
                "provider": "designer-batch",
                "evidence": "available",
                "provider_reason": "designer batch DumpIB capability is verified",
                "artifact_kind": "dt",
                "output": "/tmp/base.dt",
                "published": true,
                "execution": {"status": "succeeded"}
            })
        );
    }
}
