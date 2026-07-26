use std::collections::{BTreeSet, HashMap};

use thiserror::Error;

use crate::contracts::events::{EventEnvelope, EventType};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReplayError {
    #[error("invalid event: {0}")]
    Invalid(String),
    #[error("duplicate event id: {0}")]
    Duplicate(String),
    #[error("missing dependency: {0}")]
    Missing(String),
    #[error("event dependency cycle")]
    Cycle,
    #[error("stream predecessor mismatch: {0}")]
    Predecessor(String),
    #[error("event source contract mismatch: {0}")]
    Source(String),
    #[error("event transition invalid: {0}")]
    Transition(String),
}

pub fn topological(events: &[EventEnvelope]) -> Result<Vec<EventEnvelope>, ReplayError> {
    for event in events {
        event
            .validate()
            .map_err(|error| ReplayError::Invalid(error.to_string()))?;
    }
    let by_id = events
        .iter()
        .map(|event| (event.event_id.clone(), event))
        .collect::<HashMap<_, _>>();
    if by_id.len() != events.len() {
        return Err(ReplayError::Duplicate("eventId".to_string()));
    }
    validate_streams(events, &by_id)?;
    let mut indegree = HashMap::<String, usize>::new();
    let mut children = HashMap::<String, Vec<String>>::new();
    for event in events {
        let dependencies = event
            .predecessor_event_id
            .iter()
            .chain(event.source_event_ids.iter())
            .collect::<BTreeSet<_>>();
        for dependency in &dependencies {
            if !by_id.contains_key(*dependency) {
                return Err(ReplayError::Missing((*dependency).clone()));
            }
            children
                .entry((*dependency).clone())
                .or_default()
                .push(event.event_id.clone());
        }
        indegree.insert(event.event_id.clone(), dependencies.len());
    }
    let mut ready = BTreeSet::<(u64, String)>::new();
    for event in events {
        if indegree[&event.event_id] == 0 {
            ready.insert((event.created_at_ms, event.event_id.clone()));
        }
    }
    let mut ordered = Vec::with_capacity(events.len());
    while let Some((_, id)) = ready.pop_first() {
        let event = by_id[&id];
        validate_sources(event, &by_id)?;
        ordered.push(event.clone());
        for child in children.get(&id).into_iter().flatten() {
            let value = indegree.get_mut(child).expect("known child");
            *value -= 1;
            if *value == 0 {
                let child_event = by_id[child];
                ready.insert((child_event.created_at_ms, child.clone()));
            }
        }
    }
    if ordered.len() != events.len() {
        return Err(ReplayError::Cycle);
    }
    Ok(ordered)
}

fn validate_streams(
    events: &[EventEnvelope],
    by_id: &HashMap<String, &EventEnvelope>,
) -> Result<(), ReplayError> {
    let mut streams = HashMap::<(String, String), Vec<&EventEnvelope>>::new();
    for event in events {
        streams
            .entry((
                format!("{:?}", event.aggregate.kind),
                event.aggregate.id.clone(),
            ))
            .or_default()
            .push(event);
    }
    for stream in streams.values() {
        let roots = stream
            .iter()
            .filter(|event| event.predecessor_event_id.is_none())
            .copied()
            .collect::<Vec<_>>();
        if roots.len() != 1 {
            return Err(ReplayError::Predecessor(stream[0].event_id.clone()));
        }
        let mut child = HashMap::<String, &EventEnvelope>::new();
        for event in stream {
            let Some(predecessor_id) = &event.predecessor_event_id else {
                continue;
            };
            let predecessor = by_id
                .get(predecessor_id)
                .ok_or_else(|| ReplayError::Missing(predecessor_id.clone()))?;
            if predecessor.aggregate != event.aggregate
                || child.insert(predecessor_id.clone(), event).is_some()
            {
                return Err(ReplayError::Predecessor(event.event_id.clone()));
            }
        }
        let mut current = roots[0];
        let mut visited = 1;
        if !predecessor_allowed(current.event_type, None) {
            return Err(ReplayError::Transition(current.event_id.clone()));
        }
        while let Some(next) = child.get(&current.event_id) {
            if !predecessor_allowed(next.event_type, Some(current.event_type)) {
                return Err(ReplayError::Transition(next.event_id.clone()));
            }
            current = next;
            visited += 1;
        }
        if visited != stream.len() {
            return Err(ReplayError::Predecessor(current.event_id.clone()));
        }
    }
    Ok(())
}

fn predecessor_allowed(event: EventType, previous: Option<EventType>) -> bool {
    use EventType::*;
    match event {
        RequestAccepted
        | RequestClaimGranted
        | SlotLeaseGranted
        | RuntimeOwnershipGranted
        | RuntimeOwnershipAdopted
        | SessionOperationClaimGranted
        | PersistedSessionLeaseGranted
        | SessionRuntimeOwnershipGranted
        | SessionRuntimeOwnershipAdopted
        | ArtifactClaimEstablished
        | ReleaseStarted
        | RuntimeTakeoverProven => previous.is_none(),
        SessionBindingEstablished => previous.is_none(),
        RequestClaimRenewed => matches!(previous, Some(RequestClaimGranted | RequestClaimRenewed)),
        SessionOperationClaimRenewed => matches!(
            previous,
            Some(SessionOperationClaimGranted | SessionOperationClaimRenewed)
        ),
        RequestClaimReleased => matches!(previous, Some(RequestClaimGranted | RequestClaimRenewed)),
        SessionOperationClaimReleased => matches!(
            previous,
            Some(SessionOperationClaimGranted | SessionOperationClaimRenewed)
        ),
        SlotLeaseRenewed => matches!(previous, Some(SlotLeaseGranted | SlotLeaseRenewed)),
        SlotLeaseReleased => matches!(
            previous,
            Some(SlotLeaseGranted | SlotLeaseRenewed | PersistedSessionLeaseGranted)
        ),
        RuntimeOwnershipRenewed => matches!(
            previous,
            Some(RuntimeOwnershipGranted | RuntimeOwnershipAdopted | RuntimeOwnershipRenewed)
        ),
        RuntimeOwnershipReleased => matches!(
            previous,
            Some(
                RuntimeOwnershipGranted
                    | RuntimeOwnershipAdopted
                    | RuntimeOwnershipRenewed
                    | SessionRuntimeOwnershipGranted
                    | SessionRuntimeOwnershipAdopted
                    | RuntimeTakeoverProven
            )
        ),
        HostAttachmentsStaged => previous == Some(RequestAccepted),
        RootCaptureStarted => previous == Some(HostAttachmentsStaged),
        RootCaptureObserved | RootCaptureFailed => previous == Some(RootCaptureStarted),
        ModelSelectionStarted => previous == Some(RootCaptureObserved),
        ModelSelectionVerified | ModelSelectionFailed => previous == Some(ModelSelectionStarted),
        SlotAttachmentsMaterialized => previous == Some(ModelSelectionVerified),
        UploadStarted => matches!(previous, Some(SlotAttachmentsMaterialized | UploadCleared)),
        UploadMismatchObserved => previous == Some(UploadStarted),
        UploadCleared => previous == Some(UploadMismatchObserved),
        UploadCompleted => previous == Some(UploadStarted),
        UploadFailed => matches!(previous, Some(UploadStarted | UploadMismatchObserved)),
        SendClickArmed => previous == Some(UploadCompleted),
        SendClicked | SendReconciled | SendUncertain | SendFailed => {
            previous == Some(SendClickArmed)
        }
        TurnStartConfirmed => matches!(previous, Some(SendClicked | SendReconciled)),
        RunningProjected => previous == Some(TurnStartConfirmed),
        PollStarted => previous.is_some(),
        PollProgress => matches!(previous, Some(PollStarted | PollProgress)),
        PollFailed | AnswerTerminal => matches!(previous, Some(PollStarted | PollProgress)),
        TerminalPersisted => previous == Some(AnswerTerminal),
        OutputPublished | OutputPublishFailed => previous == Some(TerminalPersisted),
        AllocationCandidateObserved
        | AllocationExhausted
        | SlotHealthProbeStarted
        | SnapshotPublished
        | SnapshotRejected
        | QaMatrixRecorded
        | QaRepeatRecorded
        | QaCountersReset => previous.is_none() || previous.is_some(),
        SlotHealthObserved => previous == Some(SlotHealthProbeStarted),
        SessionRebindStarted => previous.is_none() || previous.is_some(),
        SessionRebound => previous == Some(SessionRebindStarted),
        SessionHydrationObserved => {
            matches!(previous, Some(SessionRebound | SessionHydrationObserved))
        }
        SessionHydrated => matches!(previous, Some(SessionRebound | SessionHydrationObserved)),
        SessionOperationFailed => previous.is_none() || previous.is_some(),
        ArtifactControlsAbsent | ArtifactControlsDiscovered => {
            previous == Some(ArtifactClaimEstablished)
        }
        ArtifactDownloadAttemptConsumed => matches!(
            previous,
            Some(ArtifactControlsDiscovered | ArtifactDownloadCompleted)
        ),
        ArtifactRecoveryCandidateObserved => previous == Some(ArtifactDownloadAttemptConsumed),
        ArtifactDownloadCompleted => matches!(
            previous,
            Some(ArtifactDownloadAttemptConsumed | ArtifactRecoveryCandidateObserved)
        ),
        ArtifactClaimCompleted => matches!(
            previous,
            Some(ArtifactControlsAbsent | ArtifactDownloadCompleted)
        ),
        ArtifactClaimFailed => matches!(
            previous,
            Some(
                ArtifactClaimEstablished
                    | ArtifactControlsAbsent
                    | ArtifactControlsDiscovered
                    | ArtifactDownloadAttemptConsumed
                    | ArtifactRecoveryCandidateObserved
            )
        ),
        ReleaseEvidencePreserved => previous == Some(ReleaseStarted),
        RuntimeStopStarted | RuntimeStopSkipped => previous == Some(ReleaseEvidencePreserved),
        RuntimeStopped | RuntimeStopFailed => previous == Some(RuntimeStopStarted),
        ReleaseCleanupStarted => matches!(
            previous,
            Some(RuntimeStopped | RuntimeStopFailed | RuntimeStopSkipped)
        ),
        ReleaseCleanupFailed => previous == Some(ReleaseCleanupStarted),
        ReleaseCleanupCommitted => previous == Some(ReleaseCleanupStarted),
        ReleaseCooldownBlocked => previous == Some(ReleaseCleanupCommitted),
        ReleaseFinalized => matches!(
            previous,
            Some(ReleaseCleanupCommitted | ReleaseCooldownBlocked)
        ),
        SlotStandbyWritten | CooldownCleared => previous.is_none() || previous.is_some(),
    }
}

fn validate_sources(
    event: &EventEnvelope,
    by_id: &HashMap<String, &EventEnvelope>,
) -> Result<(), ReplayError> {
    let sources = event
        .source_event_ids
        .iter()
        .map(|id| {
            by_id
                .get(id)
                .copied()
                .ok_or_else(|| ReplayError::Missing(id.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source_types = sources
        .iter()
        .map(|source| source.event_type)
        .collect::<Vec<_>>();
    if let Some(expected) = exact_source_types(event.event_type) {
        if source_types != expected {
            return Err(ReplayError::Source(event.event_id.clone()));
        }
        return validate_source_payloads(event, &sources);
    }
    validate_variable_sources(event, &source_types)?;
    validate_source_payloads(event, &sources)
}

fn exact_source_types(event: EventType) -> Option<Vec<EventType>> {
    use EventType::*;
    let types = match event {
        RequestAccepted | SnapshotPublished | SnapshotRejected | QaMatrixRecorded
        | QaRepeatRecorded | QaCountersReset => vec![],
        RequestClaimGranted => vec![RequestAccepted],
        HostAttachmentsStaged => vec![RequestClaimGranted],
        AllocationCandidateObserved => vec![HostAttachmentsStaged],
        RuntimeOwnershipGranted | RuntimeOwnershipAdopted => vec![SlotLeaseGranted],
        RootCaptureStarted => vec![SlotHealthObserved],
        RootCaptureObserved | RootCaptureFailed => vec![RootCaptureStarted],
        ModelSelectionStarted => vec![RootCaptureObserved],
        ModelSelectionVerified | ModelSelectionFailed => vec![ModelSelectionStarted],
        SlotAttachmentsMaterialized => vec![ModelSelectionVerified],
        UploadMismatchObserved => vec![UploadStarted],
        UploadCleared => vec![UploadMismatchObserved],
        SendClickArmed => vec![UploadCompleted],
        SendClicked | SendReconciled | SendUncertain | SendFailed => vec![SendClickArmed],
        TurnStartConfirmed => match event {
            TurnStartConfirmed => return None,
            _ => unreachable!(),
        },
        SessionBindingEstablished => vec![TurnStartConfirmed, RootCaptureObserved],
        RunningProjected => vec![SessionBindingEstablished],
        PersistedSessionLeaseGranted => vec![SessionOperationClaimGranted],
        SessionRebound => vec![SessionRebindStarted],
        PollStarted => vec![SessionHydrated],
        ArtifactControlsAbsent | ArtifactControlsDiscovered => vec![ArtifactClaimEstablished],
        ArtifactRecoveryCandidateObserved => vec![ArtifactDownloadAttemptConsumed],
        ReleaseEvidencePreserved => vec![ReleaseStarted],
        RuntimeStopped | RuntimeStopFailed => vec![RuntimeStopStarted],
        RuntimeStopSkipped => vec![ReleaseEvidencePreserved],
        ReleaseCleanupStarted => return None,
        ReleaseCleanupFailed => vec![ReleaseCleanupStarted],
        CooldownCleared => return None,
        _ => return None,
    };
    Some(types)
}

fn validate_variable_sources(
    event: &EventEnvelope,
    source_types: &[EventType],
) -> Result<(), ReplayError> {
    use EventType::*;
    let valid = match event.event_type {
        RequestClaimRenewed | SessionOperationClaimRenewed => {
            source_types.len() == 1
                && matches!(
                    source_types[0],
                    RequestClaimGranted
                        | RequestClaimRenewed
                        | SessionOperationClaimGranted
                        | SessionOperationClaimRenewed
                )
        }
        AllocationExhausted => {
            source_types.len() == 10
                && source_types
                    .iter()
                    .all(|kind| *kind == AllocationCandidateObserved)
        }
        SlotLeaseGranted => {
            source_types.len() >= 2
                && source_types[0] == RequestClaimGranted
                && source_types[1..]
                    .iter()
                    .all(|kind| *kind == AllocationCandidateObserved)
        }
        SlotLeaseRenewed => {
            source_types.len() == 1
                && matches!(source_types[0], SlotLeaseGranted | SlotLeaseRenewed)
        }
        RuntimeOwnershipRenewed => {
            source_types.len() == 1
                && matches!(
                    source_types[0],
                    RuntimeOwnershipGranted | RuntimeOwnershipAdopted | RuntimeOwnershipRenewed
                )
        }
        SlotHealthProbeStarted => {
            source_types.len() == 2
                && matches!(
                    source_types[0],
                    SlotLeaseGranted | PersistedSessionLeaseGranted
                )
                && matches!(
                    source_types[1],
                    RuntimeOwnershipGranted
                        | RuntimeOwnershipAdopted
                        | SessionRuntimeOwnershipGranted
                        | SessionRuntimeOwnershipAdopted
                )
        }
        SlotHealthObserved => source_types == [SlotHealthProbeStarted],
        UploadStarted => {
            source_types.len() == 1
                && matches!(source_types[0], SlotAttachmentsMaterialized | UploadCleared)
        }
        UploadCompleted => source_types == [UploadStarted],
        UploadFailed => {
            source_types.len() == 1
                && matches!(source_types[0], UploadStarted | UploadMismatchObserved)
        }
        TurnStartConfirmed => {
            source_types.len() == 1 && matches!(source_types[0], SendClicked | SendReconciled)
        }
        SessionOperationClaimGranted => {
            source_types.is_empty() || source_types == [RunningProjected]
        }
        SessionRuntimeOwnershipGranted | SessionRuntimeOwnershipAdopted => {
            source_types == [SessionOperationClaimGranted, PersistedSessionLeaseGranted]
        }
        SessionRebindStarted => {
            source_types.len() == 3
                && matches!(
                    source_types[0],
                    SessionOperationClaimGranted | SessionOperationClaimRenewed
                )
                && matches!(
                    source_types[1],
                    SlotLeaseGranted | SlotLeaseRenewed | PersistedSessionLeaseGranted
                )
                && matches!(
                    source_types[2],
                    RuntimeOwnershipGranted
                        | RuntimeOwnershipAdopted
                        | RuntimeOwnershipRenewed
                        | SessionRuntimeOwnershipGranted
                        | SessionRuntimeOwnershipAdopted
                )
        }
        SessionHydrationObserved | SessionHydrated => {
            source_types.len() == 1
                && matches!(source_types[0], SessionRebound | SessionHydrationObserved)
        }
        SessionOperationFailed => source_types.len() == 1,
        PollProgress | PollFailed => {
            source_types.len() == 1 && matches!(source_types[0], PollStarted | PollProgress)
        }
        AnswerTerminal => source_types.is_empty() || source_types == [SessionHydrated],
        ArtifactClaimEstablished => {
            source_types.len() == 1 && matches!(source_types[0], AnswerTerminal | SessionHydrated)
        }
        ArtifactDownloadAttemptConsumed => {
            source_types.len() == 1
                && matches!(
                    source_types[0],
                    ArtifactControlsDiscovered | ArtifactDownloadCompleted
                )
        }
        ArtifactDownloadCompleted => {
            source_types.len() == 1
                && matches!(
                    source_types[0],
                    ArtifactDownloadAttemptConsumed | ArtifactRecoveryCandidateObserved
                )
        }
        ArtifactClaimCompleted => {
            !source_types.is_empty()
                && source_types
                    .iter()
                    .all(|kind| matches!(kind, ArtifactControlsAbsent | ArtifactDownloadCompleted))
        }
        ArtifactClaimFailed => source_types.is_empty(),
        TerminalPersisted => {
            !source_types.is_empty()
                && source_types[0] == AnswerTerminal
                && source_types[1..]
                    .iter()
                    .all(|kind| matches!(kind, ArtifactClaimCompleted | ArtifactClaimFailed))
        }
        OutputPublished | OutputPublishFailed => source_types == [TerminalPersisted],
        ReleaseStarted => release_sources_valid(source_types),
        RuntimeTakeoverProven => {
            source_types.len() == 2 && source_types[1] == ReleaseEvidencePreserved
        }
        RuntimeStopStarted => source_types.is_empty() || source_types == [RuntimeTakeoverProven],
        ReleaseCleanupStarted => {
            source_types.len() == 1
                && matches!(
                    source_types[0],
                    RuntimeStopped | RuntimeStopFailed | RuntimeStopSkipped
                )
        }
        SessionOperationClaimReleased => source_types == [ReleaseCleanupStarted],
        RequestClaimReleased => {
            source_types == [ReleaseCleanupStarted]
                || source_types == [ReleaseCleanupStarted, SessionOperationClaimReleased]
        }
        SlotLeaseReleased => {
            !source_types.is_empty()
                && source_types.iter().all(|kind| {
                    matches!(
                        kind,
                        RequestClaimReleased
                            | SessionOperationClaimReleased
                            | ReleaseCleanupStarted
                    )
                })
        }
        RuntimeOwnershipReleased => {
            source_types.iter().any(|kind| {
                matches!(
                    kind,
                    RuntimeStopped | RuntimeStopFailed | RuntimeStopSkipped
                )
            }) && source_types.iter().all(|kind| {
                matches!(
                    kind,
                    SlotLeaseReleased | RuntimeStopped | RuntimeStopFailed | RuntimeStopSkipped
                )
            })
        }
        ReleaseCleanupCommitted => source_types.iter().all(|kind| {
            matches!(
                kind,
                RequestClaimReleased
                    | SessionOperationClaimReleased
                    | SlotLeaseReleased
                    | RuntimeOwnershipReleased
            )
        }),
        SlotStandbyWritten => source_types == [ReleaseCleanupCommitted],
        ReleaseCooldownBlocked => source_types == [SlotStandbyWritten],
        CooldownCleared => source_types == [SlotStandbyWritten],
        ReleaseFinalized => {
            source_types.is_empty()
                || source_types == [SlotStandbyWritten]
                || source_types == [SlotStandbyWritten, CooldownCleared]
        }
        _ => false,
    };
    valid
        .then_some(())
        .ok_or_else(|| ReplayError::Source(event.event_id.clone()))
}

fn validate_source_payloads(
    event: &EventEnvelope,
    sources: &[&EventEnvelope],
) -> Result<(), ReplayError> {
    use EventType::*;
    let valid = match event.event_type {
        RequestClaimGranted => same(event, "requestId", sources[0], "requestId"),
        RequestClaimRenewed | SessionOperationClaimRenewed => {
            same(event, "claimId", sources[0], "claimId")
                && same_u64(event, "claimGeneration", sources[0], "claimGeneration")
                || (same(event, "claimId", sources[0], "claimId")
                    && number(sources[0], "claimGeneration").is_none()
                    && number(event, "claimGeneration") == Some(1))
        }
        HostAttachmentsStaged => same(event, "requestId", sources[0], "requestId"),
        AllocationCandidateObserved => same(event, "requestId", sources[0], "requestId"),
        AllocationExhausted => validate_allocation_exhausted(event, sources),
        SlotLeaseGranted => validate_lease_sources(event, sources),
        SlotLeaseRenewed => {
            same(event, "leaseId", sources[0], "leaseId")
                && same_u64(event, "leaseGeneration", sources[0], "leaseGeneration")
        }
        RuntimeOwnershipGranted | RuntimeOwnershipAdopted => {
            same(event, "leaseId", sources[0], "leaseId")
                && same(event, "slotId", sources[0], "slotId")
        }
        RuntimeOwnershipRenewed => {
            same(event, "runtimeOwnerId", sources[0], "runtimeOwnerId")
                && same_u64(event, "ownerGeneration", sources[0], "ownerGeneration")
        }
        RootCaptureStarted => {
            same(event, "slotId", sources[0], "slotId")
                && matches!(
                    text(sources[0], "healthStatus"),
                    Some("ready" | "ready_model_correction_required")
                )
        }
        RootCaptureObserved | RootCaptureFailed => same_operation(
            event,
            "captureOperationId",
            sources[0],
            "captureOperationId",
        ),
        ModelSelectionStarted => event.aggregate.id == sources[0].aggregate.id,
        ModelSelectionVerified | ModelSelectionFailed => {
            same_operation(event, "modelOperationId", sources[0], "modelOperationId")
        }
        SlotAttachmentsMaterialized => event.aggregate.id == sources[0].aggregate.id,
        UploadStarted => event.aggregate.id == sources[0].aggregate.id,
        UploadMismatchObserved | UploadCompleted | UploadFailed => {
            same_operation(event, "uploadAttemptId", sources[0], "uploadAttemptId")
                && event.aggregate.id == sources[0].aggregate.id
        }
        UploadCleared => {
            same_operation(event, "uploadAttemptId", sources[0], "uploadAttemptId")
                && event.aggregate.id == sources[0].aggregate.id
        }
        SendClickArmed => {
            same_operation(event, "uploadAttemptId", sources[0], "uploadAttemptId")
                && event.aggregate.id == sources[0].aggregate.id
        }
        SendClicked | SendReconciled | SendUncertain | SendFailed => {
            same_operation(event, "sendAttemptId", sources[0], "sendAttemptId")
                && event.aggregate.id == sources[0].aggregate.id
        }
        TurnStartConfirmed => validate_turn_start_source(event, sources[0]),
        SessionBindingEstablished => validate_session_binding_sources(event, sources),
        RunningProjected => {
            same(event, "sessionId", sources[0], "sessionId")
                && same(event, "sessionBindingId", sources[0], "sessionBindingId")
        }
        PersistedSessionLeaseGranted => {
            same(event, "claimId", sources[0], "claimId")
                && same(event, "slotId", sources[0], "expectedSlotId")
                && same(event, "cohort", sources[0], "expectedCohort")
        }
        SessionRuntimeOwnershipGranted | SessionRuntimeOwnershipAdopted => {
            sources.len() == 2
                && same(event, "sessionId", sources[0], "sessionId")
                && same(event, "leaseId", sources[1], "leaseId")
                && same(event, "slotId", sources[1], "slotId")
        }
        SessionRebindStarted => validate_rebind_sources(event, sources),
        SessionRebound => same(event, "sessionId", sources[0], "sessionId"),
        SessionHydrationObserved | SessionHydrated => {
            same(event, "sessionId", sources[0], "sessionId")
        }
        PollStarted => same(event, "sessionId", sources[0], "sessionId"),
        PollProgress | PollFailed => {
            same_operation(event, "pollAttemptId", sources[0], "pollAttemptId")
        }
        AnswerTerminal => {
            sources.is_empty()
                || (same(event, "sessionId", sources[0], "sessionId")
                    && text(event, "requestId") == event.request_id.as_deref())
        }
        ArtifactControlsAbsent | ArtifactControlsDiscovered => {
            same(event, "artifactClaimId", sources[0], "artifactClaimId")
        }
        ArtifactDownloadAttemptConsumed => {
            same(event, "artifactClaimId", sources[0], "artifactClaimId")
        }
        ArtifactRecoveryCandidateObserved | ArtifactDownloadCompleted => {
            same(event, "artifactClaimId", sources[0], "artifactClaimId")
                && same_operation(event, "attemptId", sources[0], "attemptId")
        }
        ArtifactClaimCompleted => sources
            .iter()
            .all(|source| same(event, "artifactClaimId", source, "artifactClaimId")),
        TerminalPersisted => validate_terminal_sources(event, sources),
        OutputPublished | OutputPublishFailed => event.aggregate.id == sources[0].aggregate.id,
        ReleaseStarted => validate_release_started_sources(event, sources),
        ReleaseEvidencePreserved => same(event, "releaseId", sources[0], "releaseId"),
        RuntimeTakeoverProven => {
            sources.len() == 2
                && text(event, "priorOwnerId") == Some(sources[0].aggregate.id.as_str())
                && same(event, "releaseId", sources[1], "releaseId")
        }
        RuntimeStopStarted => {
            sources.is_empty()
                || (same(event, "runtimeOwnerId", sources[0], "newOwnerId")
                    && same_u64(event, "ownerGeneration", sources[0], "newGeneration"))
        }
        RuntimeStopped | RuntimeStopFailed => {
            same(event, "releaseId", sources[0], "releaseId")
                && same(event, "runtimeOwnerId", sources[0], "runtimeOwnerId")
                && same_u64(event, "ownerGeneration", sources[0], "ownerGeneration")
        }
        RuntimeStopSkipped => same(event, "releaseId", sources[0], "releaseId"),
        ReleaseCleanupStarted | ReleaseCleanupFailed => {
            same(event, "releaseId", sources[0], "releaseId")
        }
        RequestClaimReleased | SessionOperationClaimReleased => {
            same(event, "releaseId", sources[0], "releaseId")
        }
        SlotLeaseReleased => sources
            .iter()
            .all(|source| same(event, "releaseId", source, "releaseId")),
        RuntimeOwnershipReleased => sources.iter().all(|source| {
            text(source, "releaseId").is_none() || same(event, "releaseId", source, "releaseId")
        }),
        ReleaseCleanupCommitted => sources
            .iter()
            .all(|source| same(event, "releaseId", source, "releaseId")),
        SlotStandbyWritten | ReleaseCooldownBlocked => {
            same(event, "releaseId", sources[0], "releaseId")
        }
        CooldownCleared => {
            same(event, "slotId", sources[0], "slotId")
                && number(sources[0], "cooldownUntilMs")
                    .is_some_and(|until| number(event, "clearedAtMs").is_some_and(|at| at >= until))
        }
        ReleaseFinalized => validate_release_finalized_sources(event, sources),
        _ => true,
    };
    valid
        .then_some(())
        .ok_or_else(|| ReplayError::Source(event.event_id.clone()))
}

fn validate_allocation_exhausted(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    sources.len() == 10
        && sources.iter().enumerate().all(|(index, source)| {
            number(source, "scanOrdinal") == Some(index as u64)
                && text(source, "decision") == Some("skip")
                && same(event, "requestId", source, "requestId")
        })
}

fn validate_lease_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    sources.len() >= 2
        && text(event, "claimId") == Some(sources[0].aggregate.id.as_str())
        && event.request_id == sources[0].request_id
        && sources[1..].iter().enumerate().all(|(index, source)| {
            number(source, "scanOrdinal") == Some(index as u64)
                && event.request_id == source.request_id
        })
        && sources.last().is_some_and(|source| {
            text(source, "decision") == Some("grantable")
                && same(event, "slotId", source, "slotId")
                && same(event, "cohort", source, "cohort")
                && same_u64(event, "cohortCursorBefore", source, "cohortCursorBefore")
                && same_u64(event, "withinCursorBefore", source, "withinCursorBefore")
        })
}

fn validate_session_binding_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    sources.len() == 2
        && same(event, "sessionId", sources[0], "sessionId")
        && same(event, "conversationUrl", sources[0], "conversationUrl")
        && text(event, "runtimeOwnerId")
            == sources[1]
                .payload
                .get("pageBinding")
                .and_then(|value| value.get("runtimeOwnerId"))
                .and_then(serde_json::Value::as_str)
        && text(event, "slotId")
            == sources[1]
                .payload
                .get("pageBinding")
                .and_then(|value| value.get("slotId"))
                .and_then(serde_json::Value::as_str)
}

fn validate_turn_start_source(event: &EventEnvelope, source: &EventEnvelope) -> bool {
    let receipt_key = match source.event_type {
        EventType::SendClicked => "postClickReceipt",
        EventType::SendReconciled => "reconciledReceipt",
        _ => return false,
    };
    let receipt = source.payload.get(receipt_key);
    event.aggregate.id == source.aggregate.id
        && text(event, "sessionId")
            == receipt
                .and_then(|value| value.get("sessionId"))
                .and_then(serde_json::Value::as_str)
        && text(event, "conversationUrl")
            == receipt
                .and_then(|value| value.get("conversationUrl"))
                .and_then(serde_json::Value::as_str)
        && text(event, "userTurnId")
            == receipt
                .and_then(|value| value.get("userTurnId"))
                .and_then(serde_json::Value::as_str)
        && text(event, "assistantTurnId")
            == receipt
                .and_then(|value| value.get("assistantTurnId"))
                .and_then(serde_json::Value::as_str)
}

fn validate_rebind_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    let claim = event.payload.get("sessionOperationClaimId");
    let claim_matches = claim.is_some_and(serde_json::Value::is_null)
        || text(event, "sessionOperationClaimId") == Some(sources[0].aggregate.id.as_str());
    sources.len() == 3
        && claim_matches
        && same(event, "sessionId", sources[0], "sessionId")
        && text(sources[0], "expectedSlotId") == text(sources[1], "slotId")
        && text(sources[1], "slotId") == text(sources[2], "slotId")
}

fn validate_terminal_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    !sources.is_empty()
        && text(event, "answerTerminalEventId") == Some(sources[0].event_id.as_str())
        && event.request_id == sources[0].request_id
        && event
            .payload
            .get("artifactClaimEventIds")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|ids| {
                ids.len() == sources.len() - 1
                    && ids.iter().zip(&sources[1..]).all(|(id, source)| {
                        id.as_str() == Some(source.event_id.as_str())
                            && source.request_id == event.request_id
                    })
            })
}

fn validate_release_started_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    let reason = text(event, "reason");
    if reason == Some("release.explicit") {
        return validate_explicit_release_source_order(sources);
    }
    if sources.len() != 1 {
        return false;
    }
    let expected = match sources[0].event_type {
        EventType::OutputPublished => "release.output_published",
        EventType::ArtifactClaimCompleted | EventType::ArtifactClaimFailed => {
            "release.artifact_terminal"
        }
        EventType::PollFailed => "release.poll_failed",
        EventType::UploadFailed => "release.upload_failed",
        EventType::SendUncertain => "release.send_uncertain",
        EventType::SendFailed => "release.send_failed",
        EventType::ModelSelectionFailed => "release.model_failed",
        EventType::RootCaptureFailed => "release.capture_failed",
        EventType::SessionOperationFailed => "release.session_operation_failed",
        EventType::AllocationExhausted => "release.allocation_exhausted",
        EventType::SlotHealthObserved => "release.readiness_failed",
        EventType::PollProgress | EventType::SessionHydrated => "release.nonterminal_publication",
        EventType::OutputPublishFailed => "release.output_publish_failed",
        _ => return false,
    };
    reason == Some(expected)
}

fn validate_explicit_release_source_order(sources: &[&EventEnvelope]) -> bool {
    let mut previous_rank = 0;
    !sources.is_empty()
        && sources.len() <= 4
        && sources.iter().all(|source| {
            let rank = match source.event_type {
                EventType::RequestClaimGranted | EventType::RequestClaimRenewed => 1,
                EventType::SessionOperationClaimGranted
                | EventType::SessionOperationClaimRenewed => 2,
                EventType::SlotLeaseGranted
                | EventType::SlotLeaseRenewed
                | EventType::PersistedSessionLeaseGranted => 3,
                EventType::RuntimeOwnershipGranted
                | EventType::RuntimeOwnershipAdopted
                | EventType::RuntimeOwnershipRenewed
                | EventType::SessionRuntimeOwnershipGranted
                | EventType::SessionRuntimeOwnershipAdopted
                | EventType::RuntimeTakeoverProven => 4,
                _ => return false,
            };
            let ordered = rank > previous_rank;
            previous_rank = rank;
            ordered
        })
}

fn validate_release_finalized_sources(event: &EventEnvelope, sources: &[&EventEnvelope]) -> bool {
    match sources {
        [] => text(event, "finalStatus") == Some("resources_released_no_slot"),
        [standby] => same(event, "releaseId", standby, "releaseId"),
        [standby, cleared] => {
            same(event, "releaseId", standby, "releaseId")
                && same(standby, "slotId", cleared, "slotId")
                && text(event, "finalStatus") == Some("allocatable")
        }
        _ => false,
    }
}

fn text<'a>(event: &'a EventEnvelope, key: &str) -> Option<&'a str> {
    event.payload.get(key).and_then(serde_json::Value::as_str)
}

fn number(event: &EventEnvelope, key: &str) -> Option<u64> {
    event.payload.get(key).and_then(serde_json::Value::as_u64)
}

fn same(first: &EventEnvelope, first_key: &str, second: &EventEnvelope, second_key: &str) -> bool {
    text(first, first_key).is_some() && text(first, first_key) == text(second, second_key)
}

fn same_operation(
    first: &EventEnvelope,
    first_key: &str,
    second: &EventEnvelope,
    second_key: &str,
) -> bool {
    same(first, first_key, second, second_key)
}

fn same_u64(
    first: &EventEnvelope,
    first_key: &str,
    second: &EventEnvelope,
    second_key: &str,
) -> bool {
    number(first, first_key).is_some() && number(first, first_key) == number(second, second_key)
}

fn release_sources_valid(source_types: &[EventType]) -> bool {
    use EventType::*;
    if source_types.is_empty() || source_types.len() > 4 {
        return false;
    }
    source_types.iter().all(|kind| {
        matches!(
            kind,
            OutputPublished
                | PollFailed
                | PollProgress
                | SessionHydrated
                | UploadFailed
                | SendUncertain
                | SendFailed
                | OutputPublishFailed
                | ModelSelectionFailed
                | SessionOperationFailed
                | RootCaptureFailed
                | AllocationExhausted
                | ArtifactClaimCompleted
                | ArtifactClaimFailed
                | SlotHealthObserved
                | RequestClaimGranted
                | RequestClaimRenewed
                | SessionOperationClaimGranted
                | SessionOperationClaimRenewed
                | SlotLeaseGranted
                | SlotLeaseRenewed
                | PersistedSessionLeaseGranted
                | RuntimeOwnershipGranted
                | RuntimeOwnershipAdopted
                | RuntimeOwnershipRenewed
                | SessionRuntimeOwnershipGranted
                | SessionRuntimeOwnershipAdopted
        )
    })
}
