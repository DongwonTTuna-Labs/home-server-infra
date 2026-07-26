use serde_json::json;

use crate::allocator::scan::Observation;
use crate::claims::{derived_id, fencing_hash, RENEW_CADENCE_MS, RESOURCE_TTL_MS};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::projection::AllocatorRecord;
use crate::journal::MutationGuard;
use crate::session_ops::journal::{NewEvent, SessionJournal, SessionJournalError};
use crate::session_ops::provider::StatusInvocationResult;
use crate::session_ops::runtime_r13::AcquiredRuntime;
use crate::uploads::AttachmentSet;

use super::event_time;

pub struct AllocationEvents {
    pub candidates: Vec<EventEnvelope>,
}

#[allow(clippy::too_many_arguments)]
pub fn append_accepted(
    journal: &mut SessionJournal,
    request_id: &str,
    kind: &str,
    prompt_sha256: &str,
    prompt_size_bytes: u64,
    attachment_count: u8,
    artifact_expectation: &str,
    guard: &MutationGuard,
) -> Result<EventEnvelope, SessionJournalError> {
    let at = event_time(None);
    journal.append_with_guard(
        guard,
        NewEvent {
            aggregate_kind: AggregateKind::Request,
            aggregate_id: request_id.to_string(),
            event_type: EventType::RequestAccepted,
            payload: json!({
                "requestId":request_id,"kind":kind,"promptSha256":prompt_sha256,
                "promptSizeBytes":prompt_size_bytes,"attachmentCount":attachment_count,
                "artifactExpectation":artifact_expectation,"acceptedAtMs":at
            }),
            predecessor_event_id: None,
            source_event_ids: Vec::new(),
            created_at_ms: at,
        },
    )
}

pub fn append_claim(
    journal: &mut SessionJournal,
    accepted: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    fencing_token: &str,
) -> Result<EventEnvelope, SessionJournalError> {
    let at = event_time(Some(accepted));
    let claim_id = derived_id(
        "claim_",
        &json!({"requestId":request_id,"operationId":operation_id,"generation":1}),
    )?;
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::Claim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::RequestClaimGranted,
        payload: json!({
            "claimId":claim_id,"requestId":request_id,"claimGeneration":1,
            "ttlMs":RESOURCE_TTL_MS,"grantedAtMs":at,"renewAtMs":at+RENEW_CADENCE_MS,
            "expiresAtMs":at+RESOURCE_TTL_MS,"fencingTokenSha256":fencing_hash(fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: vec![accepted.event_id.clone()],
        created_at_ms: at,
    })
}

pub fn append_host_staged(
    journal: &mut SessionJournal,
    accepted: &EventEnvelope,
    claim: &EventEnvelope,
    request_id: &str,
    set: &AttachmentSet,
) -> Result<EventEnvelope, SessionJournalError> {
    let at = event_time(Some(accepted));
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.to_string(),
        event_type: EventType::HostAttachmentsStaged,
        payload: json!({"requestId":request_id,"attachmentSet":set,"stagedAtMs":at}),
        predecessor_event_id: Some(accepted.event_id.clone()),
        source_event_ids: vec![claim.event_id.clone()],
        created_at_ms: at,
    })
}

pub fn append_allocation(
    journal: &mut SessionJournal,
    staged: &EventEnvelope,
    request_id: &str,
    observations: &[Observation],
    mut predecessor_event_id: Option<String>,
) -> Result<AllocationEvents, SessionJournalError> {
    let mut candidates = Vec::with_capacity(observations.len());
    for observation in observations {
        let at = event_time(candidates.last().or(Some(staged)));
        let event = journal.append(NewEvent {
            aggregate_kind: AggregateKind::Allocator,
            aggregate_id: "allocator".to_string(),
            event_type: EventType::AllocationCandidateObserved,
            payload: json!({
                "requestId":request_id,"scanOrdinal":observation.scan_ordinal,
                "cohort":observation.cohort,"slotId":observation.slot_id,
                "cohortCursorBefore":observation.cohort_cursor_before,
                "withinCursorBefore":observation.within_cursor_before,
                "decision":observation.decision,
                "skipReason":observation.skip_reason.map(|reason| reason.as_str()),
                "observedAtMs":at
            }),
            predecessor_event_id,
            source_event_ids: vec![staged.event_id.clone()],
            created_at_ms: at,
        })?;
        predecessor_event_id = Some(event.event_id.clone());
        candidates.push(event);
    }
    Ok(AllocationEvents { candidates })
}

#[allow(clippy::too_many_arguments)]
pub fn append_lease(
    journal: &mut SessionJournal,
    claim: &EventEnvelope,
    allocation: &AllocationEvents,
    allocator: &AllocatorRecord,
    request_id: &str,
    operation_id: &str,
    fencing_token: &str,
) -> Result<EventEnvelope, SessionJournalError> {
    let selected = allocation.candidates.last().expect("selected allocation");
    let slot_id = selected.payload["slotId"].as_str().expect("validated slot");
    let cohort = selected.payload["cohort"]
        .as_str()
        .expect("validated cohort");
    let lease_id = derived_id(
        "lease_",
        &json!([
            "pr72.fresh-slot-lease.r13.v1",
            request_id,
            operation_id,
            slot_id,
            1
        ]),
    )?;
    let within_after = match cohort {
        "cohort-a" => allocator.within_cursors.cohort_a,
        "cohort-b" => allocator.within_cursors.cohort_b,
        _ => allocator.within_cursors.cohort_c,
    };
    let at = event_time(Some(selected));
    let mut sources = vec![claim.event_id.clone()];
    sources.extend(
        allocation
            .candidates
            .iter()
            .map(|event| event.event_id.clone()),
    );
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::Lease,
        aggregate_id: lease_id.clone(),
        event_type: EventType::SlotLeaseGranted,
        payload: json!({
            "leaseId":lease_id,"claimId":claim.aggregate.id,"slotId":slot_id,"cohort":cohort,
            "cohortCursorBefore":selected.payload["cohortCursorBefore"],
            "withinCursorBefore":selected.payload["withinCursorBefore"],
            "cohortCursorAfter":allocator.cohort_cursor,"withinCursorAfter":within_after,
            "leaseGeneration":1,"reason":"fresh_send","grantedAtMs":at,
            "renewAtMs":at+RENEW_CADENCE_MS,"expiresAtMs":at+RESOURCE_TTL_MS,
            "fencingTokenSha256":fencing_hash(fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: sources,
        created_at_ms: at,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn append_owner(
    journal: &mut SessionJournal,
    lease: &EventEnvelope,
    acquired: &AcquiredRuntime,
    slot_id: &str,
    fencing_token: &str,
) -> Result<EventEnvelope, SessionJournalError> {
    let at = event_time(Some(lease));
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::RuntimeOwner,
        aggregate_id: acquired.owner_id.clone(),
        event_type: EventType::RuntimeOwnershipGranted,
        payload: json!({
            "runtimeOwnerId":acquired.owner_id,"slotId":slot_id,"leaseId":lease.aggregate.id,
            "ownerGeneration":acquired.owner_generation,
            "runtimeIncarnationId":acquired.runtime_incarnation_id,
            "dockerStatus":acquired.docker_status,"startReceipt":acquired.start_receipt,
            "grantedAtMs":at,"renewAtMs":at+RENEW_CADENCE_MS,
            "expiresAtMs":at+RESOURCE_TTL_MS,"fencingTokenSha256":fencing_hash(fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: vec![lease.event_id.clone()],
        created_at_ms: at,
    })
}

pub fn append_health_probe(
    journal: &mut SessionJournal,
    slot_id: &str,
    probe_id: &str,
    retry_index: u8,
    slot_predecessor: Option<String>,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
) -> Result<EventEnvelope, SessionJournalError> {
    let at = event_time(Some(owner));
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: slot_id.to_string(),
        event_type: EventType::SlotHealthProbeStarted,
        payload: json!({"slotId":slot_id,"probeId":probe_id,"dockerStatus":"running",
            "deadlineMs":15000,"retryIndex":retry_index,"startedAtMs":at}),
        predecessor_event_id: slot_predecessor,
        source_event_ids: vec![lease.event_id.clone(), owner.event_id.clone()],
        created_at_ms: at,
    })
}

pub fn append_health_observed(
    journal: &mut SessionJournal,
    slot_id: &str,
    probe_id: &str,
    probe: &EventEnvelope,
    status: &StatusInvocationResult,
) -> Result<EventEnvelope, SessionJournalError> {
    let observed_at = event_time(Some(probe));
    let policy = crate::allocator::health::map_health(status.health_status, status.retry_after_ms);
    journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: slot_id.to_string(),
        event_type: EventType::SlotHealthObserved,
        payload: json!({
            "slotId":slot_id,"probeId":probe_id,"healthStatus":status.health_status,
            "dockerStatus":status.docker_status,"cooldownMs":policy.cooldown_ms,
            "allocatable":policy.allocatable,"evidenceRefs":[status.receipt],
            "observedAtMs":observed_at
        }),
        predecessor_event_id: Some(probe.event_id.clone()),
        source_event_ids: vec![probe.event_id.clone()],
        created_at_ms: observed_at,
    })
}
