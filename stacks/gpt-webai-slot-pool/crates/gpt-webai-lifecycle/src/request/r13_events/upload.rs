use serde_json::{json, Value};

use crate::contracts::browser::EvidenceRef;
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::session_ops::journal::{NewEvent, SessionJournal};
use crate::uploads::UploadProof;

use super::{event_time, SendEventError};

pub fn append_upload_mismatch(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    proof: &UploadProof,
) -> Result<EventEnvelope, SendEventError> {
    let at = event_time(Some(started));
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.to_string(),
        event_type: EventType::UploadMismatchObserved,
        payload: json!({
            "requestId":request_id,"uploadAttemptId":proof.upload_attempt_id,
            "uploadProof":proof,"reason":"upload.stale_chip_mismatch","observedAtMs":at
        }),
        predecessor_event_id: Some(started.event_id.clone()),
        source_event_ids: vec![started.event_id.clone()],
        created_at_ms: at,
    })?)
}

pub fn append_upload_cleared(
    journal: &mut SessionJournal,
    mismatch: &EventEnvelope,
    request_id: &str,
    upload_attempt_id: &str,
    clear_attempt_id: &str,
    cleared_chips: &[Value],
    receipt: &EvidenceRef,
) -> Result<EventEnvelope, SendEventError> {
    let at = event_time(Some(mismatch));
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.to_string(),
        event_type: EventType::UploadCleared,
        payload: json!({
            "requestId":request_id,"uploadAttemptId":upload_attempt_id,
            "clearAttemptId":clear_attempt_id,"clearedChips":cleared_chips,
            "providerReceipt":receipt,"clearedAtMs":at
        }),
        predecessor_event_id: Some(mismatch.event_id.clone()),
        source_event_ids: vec![mismatch.event_id.clone()],
        created_at_ms: at,
    })?)
}
