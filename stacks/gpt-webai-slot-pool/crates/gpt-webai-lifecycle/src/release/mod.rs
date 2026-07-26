pub mod cleanup;
pub mod ownership;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::ids::{
    validate_h256, validate_non_empty_text, validate_release_id, validate_safe_rel_path,
    validate_timestamp_ms,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseSubjectKind {
    Request,
    SessionOperation,
    Slot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReleaseReason {
    #[serde(rename = "release.output_published")]
    OutputPublished,
    #[serde(rename = "release.artifact_terminal")]
    ArtifactTerminal,
    #[serde(rename = "release.poll_failed")]
    PollFailed,
    #[serde(rename = "release.upload_failed")]
    UploadFailed,
    #[serde(rename = "release.send_uncertain")]
    SendUncertain,
    #[serde(rename = "release.send_failed")]
    SendFailed,
    #[serde(rename = "release.model_failed")]
    ModelFailed,
    #[serde(rename = "release.capture_failed")]
    CaptureFailed,
    #[serde(rename = "release.session_operation_failed")]
    SessionOperationFailed,
    #[serde(rename = "release.allocation_exhausted")]
    AllocationExhausted,
    #[serde(rename = "release.readiness_failed")]
    ReadinessFailed,
    #[serde(rename = "release.nonterminal_publication")]
    NonterminalPublication,
    #[serde(rename = "release.output_publish_failed")]
    OutputPublishFailed,
    #[serde(rename = "release.explicit")]
    Explicit,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourceKind {
    RequestClaim,
    SessionClaim,
    SlotLease,
    RuntimeOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeOutcome {
    Pending,
    Stopped,
    SkippedNotAcquired,
    SkippedOwnerAlive,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseFinalStatus {
    Allocatable,
    CooldownBlocked,
    CleanupFailed,
    StopSkippedOwnerAlive,
    ResourcesReleasedNoSlot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceManifest {
    pub path: String,
    pub sha256: String,
    pub preserved_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseStart {
    pub release_id: String,
    pub subject_kind: ReleaseSubjectKind,
    pub subject_id: String,
    pub reason: ReleaseReason,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReleaseError {
    #[error("invalid release field: {0}")]
    Invalid(&'static str),
    #[error("illegal release transition")]
    IllegalTransition,
    #[error("runtime ownership error: {0}")]
    Ownership(String),
    #[error("release.fencing_mismatch")]
    FencingMismatch,
}

impl ReleaseStart {
    pub fn validate(&self) -> Result<(), ReleaseError> {
        validate_release_id(&self.release_id).map_err(|_| ReleaseError::Invalid("releaseId"))?;
        validate_non_empty_text(&self.subject_id)
            .map_err(|_| ReleaseError::Invalid("subjectId"))?;
        validate_timestamp_ms(self.started_at_ms).map_err(|_| ReleaseError::Invalid("startedAtMs"))
    }
}

impl EvidenceManifest {
    pub fn validate(&self) -> Result<(), ReleaseError> {
        validate_safe_rel_path(&self.path)
            .map_err(|_| ReleaseError::Invalid("evidenceManifestPath"))?;
        validate_h256(&self.sha256).map_err(|_| ReleaseError::Invalid("evidenceManifestSha256"))?;
        validate_timestamp_ms(self.preserved_at_ms)
            .map_err(|_| ReleaseError::Invalid("preservedAtMs"))
    }
}
