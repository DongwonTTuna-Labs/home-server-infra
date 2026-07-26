use thiserror::Error;

use crate::contracts::browser::{validate_model_tuple, EffortProof, FailureProof, ModelProof};
use crate::contracts::ids::validate_timestamp_ms;

pub const MODEL_FAILURE_REASONS: [&str; 6] = [
    "picker.model_absent",
    "picker.effort_absent",
    "picker.control_drift",
    "picker.selection_timeout",
    "picker.reverify_mismatch",
    "capture.ambiguous",
];

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelSelectionError {
    #[error("model selection proof invalid: {0}")]
    Invalid(&'static str),
}

pub fn validate_success_proofs(
    model: &ModelProof,
    effort: &EffortProof,
) -> Result<(), ModelSelectionError> {
    validate_model_tuple(&model.requested, &effort.requested)
        .map_err(|_| ModelSelectionError::Invalid("requested tuple"))?;
    if model.requested != model.observed
        || effort.requested != effort.observed
        || !model.verified
        || !effort.verified
        || !matches!(model.selected_by.as_str(), "already_exact" | "picker")
        || !matches!(effort.selected_by.as_str(), "already_exact" | "picker")
    {
        return Err(ModelSelectionError::Invalid("proof identity"));
    }
    model
        .control
        .validate()
        .map_err(|_| ModelSelectionError::Invalid("model control"))?;
    effort
        .control
        .validate()
        .map_err(|_| ModelSelectionError::Invalid("effort control"))?;
    validate_evidence(&model.evidence_refs)?;
    validate_evidence(&effort.evidence_refs)?;
    validate_timestamp_ms(model.verified_at_ms)
        .map_err(|_| ModelSelectionError::Invalid("model verifiedAtMs"))?;
    validate_timestamp_ms(effort.verified_at_ms)
        .map_err(|_| ModelSelectionError::Invalid("effort verifiedAtMs"))
}

pub fn validate_failure_proof(proof: &FailureProof) -> Result<(), ModelSelectionError> {
    if !MODEL_FAILURE_REASONS.contains(&proof.reason.as_str()) {
        return Err(ModelSelectionError::Invalid("reason"));
    }
    validate_evidence(&proof.evidence_refs)?;
    validate_timestamp_ms(proof.failed_at_ms)
        .map_err(|_| ModelSelectionError::Invalid("failedAtMs"))
}

fn validate_evidence(
    evidence: &[crate::contracts::browser::EvidenceRef],
) -> Result<(), ModelSelectionError> {
    if !(1..=4).contains(&evidence.len()) || evidence.iter().any(|item| item.validate().is_err()) {
        return Err(ModelSelectionError::Invalid("evidenceRefs"));
    }
    let unique = evidence
        .iter()
        .map(|item| item.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    (unique.len() == evidence.len())
        .then_some(())
        .ok_or(ModelSelectionError::Invalid("duplicate evidenceRefs"))
}
