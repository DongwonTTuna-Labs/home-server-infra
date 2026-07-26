pub mod baseline;
pub mod completion;
pub mod recovery;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::browser::EvidenceRef;
use crate::contracts::ids::{
    validate_artifact_claim_id, validate_control_id, validate_h256, validate_timestamp_ms,
    validate_turn_id,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactExpectation {
    None,
    Optional,
    Required,
    Claimed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactControl {
    pub control_id: String,
    pub role: String,
    pub visible_text_hash: String,
    pub dom_path_hash: String,
    pub bounding_box_hash: String,
    pub current_turn_id: String,
    pub visible: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BottomProof {
    pub at_bottom: bool,
    pub method: String,
    pub captured_at_ms: u64,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ZeroControlProof {
    pub artifact_claim_id: String,
    pub terminal_assistant_turn_id: String,
    pub bottom_proof: BottomProof,
    pub control_count: u8,
    pub evidence_refs: Vec<EvidenceRef>,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ArtifactClaimError {
    #[error("invalid artifact claim field: {0}")]
    Invalid(&'static str),
    #[error("artifact claim transition is illegal")]
    IllegalTransition,
    #[error("artifact file I/O failed: {0}")]
    Io(String),
}

impl ArtifactControl {
    pub fn validate_for_turn(&self, turn_id: &str) -> Result<(), ArtifactClaimError> {
        valid(validate_control_id(&self.control_id), "controlId")?;
        valid(validate_h256(&self.visible_text_hash), "visibleTextHash")?;
        valid(validate_h256(&self.dom_path_hash), "domPathHash")?;
        valid(validate_h256(&self.bounding_box_hash), "boundingBoxHash")?;
        valid(validate_turn_id(&self.current_turn_id), "currentTurnId")?;
        if self.current_turn_id != turn_id
            || !self.visible
            || self.disabled
            || !matches!(self.role.as_str(), "button" | "link")
        {
            return Err(ArtifactClaimError::Invalid("control scope/state"));
        }
        Ok(())
    }
}

impl BottomProof {
    pub fn validate(&self) -> Result<(), ArtifactClaimError> {
        if !self.at_bottom
            || !matches!(
                self.method.as_str(),
                "scrollbar" | "floating_affordance" | "dom_terminal_anchor"
            )
        {
            return Err(ArtifactClaimError::Invalid("bottom proof"));
        }
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        validate_evidence(&self.evidence_refs)
    }
}

impl ZeroControlProof {
    pub fn validate_for(&self, claim_id: &str, turn_id: &str) -> Result<(), ArtifactClaimError> {
        valid(
            validate_artifact_claim_id(&self.artifact_claim_id),
            "artifactClaimId",
        )?;
        valid(
            validate_turn_id(&self.terminal_assistant_turn_id),
            "terminalAssistantTurnId",
        )?;
        self.bottom_proof.validate()?;
        validate_evidence(&self.evidence_refs)?;
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        if self.artifact_claim_id != claim_id
            || self.terminal_assistant_turn_id != turn_id
            || self.control_count != 0
        {
            return Err(ArtifactClaimError::Invalid("zero control binding"));
        }
        Ok(())
    }
}

pub(crate) fn validate_evidence(values: &[EvidenceRef]) -> Result<(), ArtifactClaimError> {
    if !(1..=4).contains(&values.len()) || values.iter().any(|item| item.validate().is_err()) {
        return Err(ArtifactClaimError::Invalid("evidenceRefs"));
    }
    let paths = values
        .iter()
        .map(|item| item.path.as_str())
        .collect::<BTreeSet<_>>();
    (paths.len() == values.len())
        .then_some(())
        .ok_or(ArtifactClaimError::Invalid("duplicate evidenceRefs"))
}

pub(crate) fn valid<T, E>(
    result: Result<T, E>,
    field: &'static str,
) -> Result<(), ArtifactClaimError> {
    result
        .map(|_| ())
        .map_err(|_| ArtifactClaimError::Invalid(field))
}
