use serde_json::json;

use crate::claims::derived_id;
use crate::config::{now_ms, SupervisorConfig};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};

use super::{
    release_claim, release_evidence_root, release_lease, write_evidence_manifest, NewEvent,
    ReleaseCompletion, SessionJournal, SessionReleaseError,
};

pub struct SessionPartialReleaseInput<'a> {
    pub config: &'a SupervisorConfig,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub session_id: &'a str,
    pub slot_id: &'a str,
    pub claim_event: &'a EventEnvelope,
    pub lease_event: Option<&'a EventEnvelope>,
    pub source_event: &'a EventEnvelope,
    pub slot_predecessor_event_id: Option<&'a str>,
    pub receipt_ids: &'a [String],
}

pub fn release_session_partial(
    journal: &mut SessionJournal,
    input: SessionPartialReleaseInput<'_>,
) -> Result<ReleaseCompletion, SessionReleaseError> {
    if input.source_event.event_type != EventType::SessionOperationFailed {
        return Err(SessionReleaseError::Contract(
            "partial session release source must be SessionOperationFailed",
        ));
    }
    let release_id = derived_id(
        "release_",
        &json!([
            "pr72.release.r13.v1",
            "session_operation",
            input.session_id,
            input.operation_id
        ]),
    )?;
    let started_at = now_ms();
    let release = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseStarted,
        payload: json!({
            "releaseId":release_id,"subjectKind":"session_operation",
            "subjectId":input.session_id,"reason":"release.session_operation_failed",
            "startedAtMs":started_at
        }),
        predecessor_event_id: None,
        source_event_ids: vec![input.source_event.event_id.clone()],
        created_at_ms: started_at,
    })?;

    let evidence_root = release_evidence_root(input.config, input.request_key, input.operation_id)?;
    let (manifest_path, manifest_sha256) = write_evidence_manifest(
        &input.config.state_root,
        &evidence_root,
        journal.event_ids(),
        input.receipt_ids,
    )?;
    let preserved_at = now_ms();
    let preserved = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseEvidencePreserved,
        payload: json!({
            "releaseId":release_id,"evidenceManifestPath":manifest_path,
            "evidenceManifestSha256":manifest_sha256,"preservedAtMs":preserved_at
        }),
        predecessor_event_id: Some(release.event_id.clone()),
        source_event_ids: vec![release.event_id],
        created_at_ms: preserved_at,
    })?;
    let skipped_at = now_ms();
    let skipped = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::RuntimeStopSkipped,
        payload: json!({
            "releaseId":release_id,"runtimeOwnerId":null,"reason":"runtime.not_acquired",
            "proofAttempt":null,"skippedAtMs":skipped_at
        }),
        predecessor_event_id: Some(preserved.event_id.clone()),
        source_event_ids: vec![preserved.event_id],
        created_at_ms: skipped_at,
    })?;
    let cleanup_at = now_ms();
    let cleanup = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseCleanupStarted,
        payload: json!({"releaseId":release_id,"startedAtMs":cleanup_at}),
        predecessor_event_id: Some(skipped.event_id.clone()),
        source_event_ids: vec![skipped.event_id],
        created_at_ms: cleanup_at,
    })?;
    let claim_released = release_claim(journal, input.claim_event, &release_id, &cleanup, true)?;
    let lease_released = input
        .lease_event
        .map(|lease| release_lease(journal, lease, &release_id, &claim_released))
        .transpose()?;
    let committed_at = now_ms();
    let mut sources = vec![claim_released.event_id.clone()];
    if let Some(lease) = lease_released.as_ref() {
        sources.push(lease.event_id.clone());
    }
    let committed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseCleanupCommitted,
        payload: json!({
            "releaseId":release_id,"requestClaimReleaseMode":"not_applicable",
            "sessionClaimReleaseMode":"released",
            "leaseReleaseMode":if lease_released.is_some(){"released"}else{"not_applicable"},
            "ownerReleaseMode":"not_applicable","committedAtMs":committed_at
        }),
        predecessor_event_id: Some(cleanup.event_id),
        source_event_ids: sources,
        created_at_ms: committed_at,
    })?;

    if lease_released.is_none() {
        let finalized_at = now_ms();
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.clone(),
            event_type: EventType::ReleaseFinalized,
            payload: json!({
                "releaseId":release_id,"finalStatus":"resources_released_no_slot",
                "allocatable":false,"finalizedAtMs":finalized_at
            }),
            predecessor_event_id: Some(committed.event_id),
            source_event_ids: Vec::new(),
            created_at_ms: finalized_at,
        })?;
        return Ok(ReleaseCompletion { stop_failed: false });
    }

    let standby_at = now_ms();
    let active_cooldown = journal
        .replay()?
        .state
        .slots
        .get(input.slot_id)
        .and_then(|slot| slot.cooldown_until_ms)
        .filter(|until| *until > standby_at);
    let standby = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: input.slot_id.to_string(),
        event_type: EventType::SlotStandbyWritten,
        payload: json!({
            "slotId":input.slot_id,"releaseId":release_id,
            "allocatable":active_cooldown.is_none(),"cooldownUntilMs":active_cooldown,
            "writtenAtMs":standby_at
        }),
        predecessor_event_id: input.slot_predecessor_event_id.map(str::to_string),
        source_event_ids: vec![committed.event_id.clone()],
        created_at_ms: standby_at,
    })?;
    if let Some(cooldown_until_ms) = active_cooldown {
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.clone(),
            event_type: EventType::ReleaseCooldownBlocked,
            payload: json!({
                "releaseId":release_id,"slotId":input.slot_id,
                "cooldownUntilMs":cooldown_until_ms,"blockedAtMs":standby_at
            }),
            predecessor_event_id: Some(committed.event_id),
            source_event_ids: vec![standby.event_id],
            created_at_ms: standby_at,
        })?;
    } else {
        let finalized_at = now_ms();
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.clone(),
            event_type: EventType::ReleaseFinalized,
            payload: json!({
                "releaseId":release_id,"finalStatus":"allocatable",
                "allocatable":true,"finalizedAtMs":finalized_at
            }),
            predecessor_event_id: Some(committed.event_id),
            source_event_ids: vec![standby.event_id],
            created_at_ms: finalized_at,
        })?;
    }
    Ok(ReleaseCompletion { stop_failed: false })
}
