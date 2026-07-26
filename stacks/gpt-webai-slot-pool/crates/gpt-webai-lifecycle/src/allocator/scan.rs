use thiserror::Error;

use crate::contracts::projection::AllocatorRecord;

use super::cursors::advance;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    Leased,
    ClaimActive,
    Cooldown,
    RuntimeOwned,
    HealthBlocked,
    StateInvalid,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::ClaimActive => "claim_active",
            Self::Cooldown => "cooldown",
            Self::RuntimeOwned => "runtime_owned",
            Self::HealthBlocked => "health_blocked",
            Self::StateInvalid => "state_invalid",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub scan_ordinal: u8,
    pub cohort: String,
    pub slot_id: String,
    pub cohort_cursor_before: u8,
    pub within_cursor_before: u8,
    pub decision: &'static str,
    pub skip_reason: Option<SkipReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanResult {
    pub observations: Vec<Observation>,
    pub granted_slot_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("candidate predicate failed: {0}")]
    Predicate(String),
}

pub fn scan(
    record: &mut AllocatorRecord,
    mut classify: impl FnMut(&str) -> Result<Option<SkipReason>, String>,
) -> Result<ScanResult, ScanError> {
    let mut observations = Vec::with_capacity(10);
    for scan_ordinal in 0..10 {
        let candidate = advance(record, scan_ordinal);
        let skip_reason = classify(&candidate.slot_id).map_err(ScanError::Predicate)?;
        let granted = skip_reason.is_none();
        observations.push(Observation {
            scan_ordinal,
            cohort: candidate.cohort,
            slot_id: candidate.slot_id.clone(),
            cohort_cursor_before: candidate.cohort_cursor_before,
            within_cursor_before: candidate.within_cursor_before,
            decision: if granted { "grantable" } else { "skip" },
            skip_reason,
        });
        if granted {
            return Ok(ScanResult {
                observations,
                granted_slot_id: Some(candidate.slot_id),
            });
        }
    }
    Ok(ScanResult {
        observations,
        granted_slot_id: None,
    })
}
