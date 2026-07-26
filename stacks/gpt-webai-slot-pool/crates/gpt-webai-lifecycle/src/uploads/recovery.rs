use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::ids::{
    validate_byte_count, validate_h256, validate_operation_id, validate_timestamp_ms,
};

use super::{
    AttachmentSet, ChipProof, UploadContractError, UploadProof, MAX_ATTACHMENTS,
    UPLOAD_PROOF_MAX_AGE_MS,
};

pub const STALE_MISMATCH_REASON: &str = "upload.stale_chip_mismatch";
pub const STALE_UNCLEARED_REASON: &str = "upload.stale_chip_uncleared";
pub const INCOMPLETE_REASON: &str = "upload.incomplete";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadOutcome {
    Completed,
    MismatchObserved { reason: &'static str },
    Failed { reason: &'static str },
}

impl ChipProof {
    pub fn validate(&self) -> Result<(), UploadContractError> {
        for value in [
            &self.chip_stable_key,
            &self.label_hash,
            &self.bounding_box_hash,
        ] {
            valid(validate_h256(value), "chip hash")?;
        }
        if let Some(value) = self.visible_size_bytes {
            valid(validate_byte_count(value), "visibleSizeBytes")?;
        }
        if let Some(value) = &self.digest {
            valid(validate_h256(value), "digest")?;
        }
        if !(1..=4).contains(&self.evidence_refs.len())
            || self
                .evidence_refs
                .iter()
                .any(|item| item.validate().is_err())
        {
            return Err(UploadContractError::Invalid("evidenceRefs"));
        }
        let paths = self
            .evidence_refs
            .iter()
            .map(|item| item.path.as_str())
            .collect::<BTreeSet<_>>();
        (paths.len() == self.evidence_refs.len())
            .then_some(())
            .ok_or(UploadContractError::Invalid("duplicate evidenceRefs"))
    }
}

impl UploadProof {
    pub fn validate(&self) -> Result<(), UploadContractError> {
        valid(
            validate_operation_id(&self.upload_attempt_id),
            "uploadAttemptId",
        )?;
        valid(
            validate_h256(&self.expected_set_sha256),
            "expectedSetSha256",
        )?;
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        if self.retry_index > 1
            || self.visible_current_chips.len() > MAX_ATTACHMENTS
            || self.stale_chips.len() > MAX_ATTACHMENTS
        {
            return Err(UploadContractError::Invalid("upload proof bounds"));
        }
        self.visible_current_chips
            .iter()
            .chain(&self.stale_chips)
            .try_for_each(ChipProof::validate)?;
        let keys = self
            .visible_current_chips
            .iter()
            .chain(&self.stale_chips)
            .map(|chip| chip.chip_stable_key.as_str())
            .collect::<BTreeSet<_>>();
        if keys.len() != self.visible_current_chips.len() + self.stale_chips.len() {
            return Err(UploadContractError::Invalid("duplicate chipStableKey"));
        }
        Ok(())
    }
}

pub fn classify_upload(
    proof: &UploadProof,
    expected: &AttachmentSet,
    observed_at_ms: u64,
) -> Result<UploadOutcome, UploadContractError> {
    proof.validate()?;
    if proof.expected_set_sha256 != expected.set_sha256 {
        return Err(UploadContractError::Invalid("expectedSetSha256"));
    }
    if !proof.stale_chips.is_empty() {
        return Ok(if proof.retry_index == 0 {
            UploadOutcome::MismatchObserved {
                reason: STALE_MISMATCH_REASON,
            }
        } else {
            UploadOutcome::Failed {
                reason: STALE_UNCLEARED_REASON,
            }
        });
    }
    if completion_proven(proof, expected, observed_at_ms)? {
        Ok(UploadOutcome::Completed)
    } else {
        Ok(UploadOutcome::Failed {
            reason: INCOMPLETE_REASON,
        })
    }
}

pub fn validate_retry_after_clear(
    mismatch: &UploadProof,
    retry: &UploadProof,
) -> Result<(), UploadContractError> {
    mismatch.validate()?;
    retry.validate()?;
    if mismatch.retry_index != 0
        || mismatch.stale_chips.is_empty()
        || retry.retry_index != 1
        || mismatch.expected_set_sha256 != retry.expected_set_sha256
        || mismatch.upload_attempt_id == retry.upload_attempt_id
    {
        return Err(UploadContractError::Invalid("upload retry sequence"));
    }
    Ok(())
}

fn completion_proven(
    proof: &UploadProof,
    expected: &AttachmentSet,
    observed_at_ms: u64,
) -> Result<bool, UploadContractError> {
    if observed_at_ms < proof.captured_at_ms
        || observed_at_ms - proof.captured_at_ms > UPLOAD_PROOF_MAX_AGE_MS
        || !proof.all_expected_complete
        || proof.visible_current_chips.len() != expected.records.len()
        || proof
            .visible_current_chips
            .iter()
            .any(|chip| !chip.complete)
    {
        return Ok(false);
    }
    let mut expected_pairs = BTreeMap::<(&str, u64), usize>::new();
    for record in &expected.records {
        *expected_pairs
            .entry((&record.source_sha256, record.size_bytes))
            .or_default() += 1;
    }
    let mut observed_pairs = BTreeMap::<(&str, u64), usize>::new();
    for chip in &proof.visible_current_chips {
        let (Some(digest), Some(size)) = (chip.digest.as_deref(), chip.visible_size_bytes) else {
            return Ok(false);
        };
        *observed_pairs.entry((digest, size)).or_default() += 1;
    }
    Ok(expected_pairs == observed_pairs)
}

fn valid<T, E>(result: Result<T, E>, field: &'static str) -> Result<(), UploadContractError> {
    result
        .map(|_| ())
        .map_err(|_| UploadContractError::Invalid(field))
}
