use crate::contracts::ids::validate_event_id;
use crate::contracts::projection::CasRecord;

use super::{fencing_hash, CasError, RENEW_CADENCE_MS, RESOURCE_TTL_MS};

pub fn renew(
    record: &CasRecord,
    generation: u16,
    fencing_token: Option<&str>,
    renewed_at_ms: u64,
    event_id: String,
) -> Result<CasRecord, CasError> {
    if validate_event_id(&event_id).is_err() {
        return Err(CasError::Invalid("renewal eventId"));
    }
    verify_active(record, generation, fencing_token, renewed_at_ms)?;
    let mut renewed = record.clone();
    renewed.renewal_revision = renewed
        .renewal_revision
        .checked_add(1)
        .ok_or(CasError::Invalid("renewalRevision"))?;
    renewed.renew_at_ms = renewed_at_ms + RENEW_CADENCE_MS;
    renewed.expires_at_ms = renewed_at_ms + RESOURCE_TTL_MS;
    renewed.last_event_id = event_id;
    Ok(renewed)
}

pub fn release(
    record: &CasRecord,
    generation: u16,
    fencing_token: Option<&str>,
    released_at_ms: u64,
    release_event_id: String,
) -> Result<CasRecord, CasError> {
    if validate_event_id(&release_event_id).is_err() || released_at_ms == 0 {
        return Err(CasError::Invalid("release event/time"));
    }
    verify_active(
        record,
        generation,
        fencing_token,
        released_at_ms.saturating_sub(1),
    )?;
    let mut released = record.clone();
    released.status = "released".to_string();
    released.released_at_ms = Some(released_at_ms);
    released.release_event_id = Some(release_event_id.clone());
    released.last_event_id = release_event_id;
    Ok(released)
}

pub fn verify_active(
    record: &CasRecord,
    generation: u16,
    fencing_token: Option<&str>,
    now_ms: u64,
) -> Result<(), CasError> {
    if record.status != "active" {
        return Err(CasError::Inactive);
    }
    if record.generation != generation {
        return Err(CasError::GenerationMismatch);
    }
    let lineage_start_ms = record
        .renew_at_ms
        .checked_sub(RENEW_CADENCE_MS)
        .ok_or(CasError::Invalid("renewal clock"))?;
    if now_ms < lineage_start_ms {
        return Err(CasError::Invalid("clock reversal"));
    }
    if now_ms >= record.expires_at_ms {
        return Err(CasError::Expired);
    }
    match (&record.fencing_token_sha256, fencing_token) {
        (Some(expected), Some(token)) if *expected == fencing_hash(token) => Ok(()),
        _ => Err(CasError::FencingMismatch),
    }
}
