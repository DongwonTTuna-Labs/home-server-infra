use std::collections::BTreeMap;

use thiserror::Error;

use crate::contracts::ids::{validate_event_id, validate_h256, validate_non_empty_text};
use crate::contracts::projection::QaCounterRecord;

pub const MATRIX_REQUIRED: u8 = 3;
pub const REPEAT_REQUIRED: u8 = 10;
pub const REPEAT_CASES: [&str; 10] = [
    "R01", "R02", "R03", "R04", "R05", "R06", "R07", "R08", "R09", "R10",
];

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum QaCounterError {
    #[error("invalid QA counter field: {0}")]
    Invalid(&'static str),
    #[error("QA counter sequence mismatch")]
    Sequence,
    #[error("QA source fingerprint changed; reset required")]
    FingerprintResetRequired,
}

pub fn empty(last_event_id: String) -> Result<QaCounterRecord, QaCounterError> {
    validate_event_id(&last_event_id).map_err(|_| QaCounterError::Invalid("lastEventId"))?;
    Ok(QaCounterRecord {
        matrix_iterations_passed: 0,
        repeat_counts: BTreeMap::new(),
        source_fingerprint: None,
        last_reset_event_id: None,
        last_event_id,
    })
}

pub fn record_matrix(
    record: &mut QaCounterRecord,
    matrix_iteration: u8,
    source_fingerprint: &str,
    cases_passed: u8,
    cases_total: u8,
    event_id: &str,
) -> Result<(), QaCounterError> {
    validate_common(source_fingerprint, event_id)?;
    ensure_fingerprint(record, source_fingerprint)?;
    if matrix_iteration == 0
        || matrix_iteration > MATRIX_REQUIRED
        || matrix_iteration != record.matrix_iterations_passed + 1
        || cases_total == 0
        || cases_total > 64
        || cases_passed != cases_total
    {
        return Err(QaCounterError::Sequence);
    }
    record.matrix_iterations_passed = matrix_iteration;
    record.source_fingerprint = Some(source_fingerprint.to_string());
    record.last_event_id = event_id.to_string();
    Ok(())
}

pub fn record_repeat(
    record: &mut QaCounterRecord,
    case_id: &str,
    repetition_index: u8,
    source_fingerprint: &str,
    passed: bool,
    event_id: &str,
) -> Result<(), QaCounterError> {
    validate_common(source_fingerprint, event_id)?;
    ensure_fingerprint(record, source_fingerprint)?;
    if !REPEAT_CASES.contains(&case_id)
        || repetition_index == 0
        || repetition_index > REPEAT_REQUIRED
        || repetition_index != record.repeat_counts.get(case_id).copied().unwrap_or(0) + 1
        || !passed
    {
        return Err(QaCounterError::Sequence);
    }
    record
        .repeat_counts
        .insert(case_id.to_string(), repetition_index);
    record.source_fingerprint = Some(source_fingerprint.to_string());
    record.last_event_id = event_id.to_string();
    Ok(())
}

pub fn reset_all(
    record: &mut QaCounterRecord,
    source_fingerprint: &str,
    event_id: &str,
) -> Result<(), QaCounterError> {
    validate_common(source_fingerprint, event_id)?;
    record.matrix_iterations_passed = 0;
    record.repeat_counts.clear();
    finish_reset(record, source_fingerprint, event_id);
    Ok(())
}

pub fn reset_case(
    record: &mut QaCounterRecord,
    case_id: &str,
    source_fingerprint: &str,
    event_id: &str,
) -> Result<(), QaCounterError> {
    validate_common(source_fingerprint, event_id)?;
    validate_non_empty_text(case_id).map_err(|_| QaCounterError::Invalid("caseId"))?;
    if !REPEAT_CASES.contains(&case_id) {
        return Err(QaCounterError::Invalid("caseId"));
    }
    record.repeat_counts.insert(case_id.to_string(), 0);
    finish_reset(record, source_fingerprint, event_id);
    Ok(())
}

pub fn complete(record: &QaCounterRecord) -> bool {
    record.matrix_iterations_passed == MATRIX_REQUIRED
        && REPEAT_CASES
            .iter()
            .all(|case| record.repeat_counts.get(*case) == Some(&REPEAT_REQUIRED))
}

fn ensure_fingerprint(
    record: &QaCounterRecord,
    source_fingerprint: &str,
) -> Result<(), QaCounterError> {
    if record
        .source_fingerprint
        .as_deref()
        .is_some_and(|current| current != source_fingerprint)
    {
        Err(QaCounterError::FingerprintResetRequired)
    } else {
        Ok(())
    }
}

fn validate_common(source_fingerprint: &str, event_id: &str) -> Result<(), QaCounterError> {
    validate_h256(source_fingerprint).map_err(|_| QaCounterError::Invalid("sourceFingerprint"))?;
    validate_event_id(event_id).map_err(|_| QaCounterError::Invalid("eventId"))
}

fn finish_reset(record: &mut QaCounterRecord, fingerprint: &str, event_id: &str) {
    record.source_fingerprint = Some(fingerprint.to_string());
    record.last_reset_event_id = Some(event_id.to_string());
    record.last_event_id = event_id.to_string();
}
