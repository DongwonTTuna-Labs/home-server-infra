use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::artifact_claims::baseline::ArtifactBaseline;
use crate::artifact_claims::completion::PlaywrightDownloadReceipt;
use crate::artifact_claims::{ArtifactControl, BottomProof, ZeroControlProof};
use crate::contracts::browser::{
    Effort, EffortProof, EvidenceRef, FailureProof, Model, ModelProof, PageBindingEcho,
    RootBindingCandidate, SessionEcho, SessionRebindExpectation,
};
use crate::contracts::ids::{
    h256, sha256_hex, validate_artifact_claim_id, validate_artifact_id, validate_binding_id,
    validate_byte_count, validate_claim_id, validate_cohort, validate_control_id,
    validate_conversation_url, validate_duration_ms, validate_event_id, validate_generation,
    validate_h256, validate_lease_id, validate_non_empty_text, validate_operation_id,
    validate_owner_id, validate_page_incarnation_id, validate_prefixed_hex, validate_release_id,
    validate_request_id, validate_run_id, validate_safe_rel_path, validate_session_id,
    validate_slot_id, validate_target_id, validate_timestamp_ms, validate_turn_id,
};
use crate::journal::canonical::canonical_bytes;
use crate::runtime::ownership::{AdoptionProof, DeadOwnerProof};
use crate::send_reconcile::{validate_receipt_pair, SendReceipt};
use crate::session_rebind::hydration::HydrationObservation;
use crate::uploads::{AttachmentSet, UploadProof};

pub const EVENT_SCHEMA: &str = "pr72.event.r13.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateKind {
    Request,
    Session,
    Slot,
    Allocator,
    Claim,
    Lease,
    RuntimeOwner,
    ArtifactClaim,
    Release,
    Qa,
}

macro_rules! event_types {
    ($($name:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub enum EventType { $($name),+ }

        impl EventType {
            pub const ALL: &'static [Self] = &[$(Self::$name),+];
            pub fn as_str(self) -> &'static str { match self { $(Self::$name => stringify!($name)),+ } }
        }
    };
}

event_types!(
    RequestAccepted,
    RequestClaimGranted,
    RequestClaimRenewed,
    HostAttachmentsStaged,
    AllocationCandidateObserved,
    AllocationExhausted,
    SlotLeaseGranted,
    SlotLeaseRenewed,
    RuntimeOwnershipGranted,
    RuntimeOwnershipAdopted,
    RuntimeOwnershipRenewed,
    SlotHealthProbeStarted,
    SlotHealthObserved,
    RootCaptureStarted,
    RootCaptureObserved,
    RootCaptureFailed,
    ModelSelectionStarted,
    ModelSelectionVerified,
    ModelSelectionFailed,
    SlotAttachmentsMaterialized,
    UploadStarted,
    UploadMismatchObserved,
    UploadCleared,
    UploadCompleted,
    UploadFailed,
    SendClickArmed,
    SendClicked,
    SendReconciled,
    SendUncertain,
    SendFailed,
    TurnStartConfirmed,
    SessionBindingEstablished,
    RunningProjected,
    SessionOperationClaimGranted,
    SessionOperationClaimRenewed,
    PersistedSessionLeaseGranted,
    SessionRuntimeOwnershipGranted,
    SessionRuntimeOwnershipAdopted,
    SessionRebindStarted,
    SessionRebound,
    SessionHydrationObserved,
    SessionHydrated,
    SessionOperationFailed,
    PollStarted,
    PollProgress,
    PollFailed,
    AnswerTerminal,
    ArtifactClaimEstablished,
    ArtifactControlsAbsent,
    ArtifactControlsDiscovered,
    ArtifactDownloadAttemptConsumed,
    ArtifactRecoveryCandidateObserved,
    ArtifactDownloadCompleted,
    ArtifactClaimCompleted,
    ArtifactClaimFailed,
    TerminalPersisted,
    OutputPublished,
    OutputPublishFailed,
    ReleaseStarted,
    ReleaseEvidencePreserved,
    RuntimeTakeoverProven,
    RuntimeStopStarted,
    RuntimeStopped,
    RuntimeStopFailed,
    RuntimeStopSkipped,
    ReleaseCleanupStarted,
    ReleaseCleanupFailed,
    SessionOperationClaimReleased,
    RequestClaimReleased,
    SlotLeaseReleased,
    RuntimeOwnershipReleased,
    ReleaseCleanupCommitted,
    SlotStandbyWritten,
    ReleaseCooldownBlocked,
    CooldownCleared,
    ReleaseFinalized,
    SnapshotPublished,
    SnapshotRejected,
    QaMatrixRecorded,
    QaRepeatRecorded,
    QaCountersReset,
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Aggregate {
    pub id: String,
    pub kind: AggregateKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Writer {
    pub host_id: String,
    pub process_id: u32,
    pub process_start_ms: u64,
    pub writer_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EventEnvelope {
    pub aggregate: Aggregate,
    pub created_at_ms: u64,
    pub event_id: String,
    pub event_type: EventType,
    pub operation_id: String,
    pub payload: Value,
    pub payload_hash: String,
    pub predecessor_event_id: Option<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub schema_version: String,
    pub source_event_ids: Vec<String>,
    pub writer: Writer,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EventError {
    #[error("invalid event field: {0}")]
    Invalid(&'static str),
    #[error("event serialization failed: {0}")]
    Serialize(String),
}

impl EventEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        aggregate: Aggregate,
        created_at_ms: u64,
        event_type: EventType,
        operation_id: impl Into<String>,
        payload: Value,
        predecessor_event_id: Option<String>,
        request_id: Option<String>,
        run_id: Option<String>,
        source_event_ids: Vec<String>,
        writer: Writer,
    ) -> Result<Self, EventError> {
        let payload_hash = h256(canonical_bytes(&payload).map_err(serialization)?);
        let mut event = Self {
            aggregate,
            created_at_ms,
            event_id: String::new(),
            event_type,
            operation_id: operation_id.into(),
            payload,
            payload_hash,
            predecessor_event_id,
            request_id,
            run_id,
            schema_version: EVENT_SCHEMA.to_string(),
            source_event_ids,
            writer,
        };
        event.event_id = format!(
            "evt_{}",
            sha256_hex(canonical_bytes(&event).map_err(serialization)?)
        );
        event.validate()?;
        Ok(event)
    }

    pub fn validate(&self) -> Result<(), EventError> {
        if self.schema_version != EVENT_SCHEMA {
            return Err(EventError::Invalid("schemaVersion"));
        }
        validate_event_id(&self.event_id).map_err(|_| EventError::Invalid("eventId"))?;
        validate_operation_id(&self.operation_id)
            .map_err(|_| EventError::Invalid("operationId"))?;
        validate_timestamp_ms(self.created_at_ms)
            .map_err(|_| EventError::Invalid("createdAtMs"))?;
        validate_writer(&self.writer)?;
        validate_optional_id(self.request_id.as_deref(), validate_request_id, "requestId")?;
        validate_optional_id(self.run_id.as_deref(), validate_run_id, "runId")?;
        if self.run_id.is_some() && self.request_id.is_none() {
            return Err(EventError::Invalid("runId without requestId"));
        }
        if self.source_event_ids.len() > 80
            || self.source_event_ids.iter().collect::<HashSet<_>>().len()
                != self.source_event_ids.len()
        {
            return Err(EventError::Invalid("sourceEventIds"));
        }
        for id in self
            .source_event_ids
            .iter()
            .chain(self.predecessor_event_id.iter())
        {
            validate_event_id(id).map_err(|_| EventError::Invalid("event dependency id"))?;
            if id == &self.event_id {
                return Err(EventError::Invalid("self dependency"));
            }
        }
        validate_payload_keys(self.event_type, self.payload_object()?)?;
        validate_payload_semantics(self)?;
        validate_aggregate(self)?;
        let payload = canonical_bytes(&self.payload).map_err(serialization)?;
        if self.payload_hash != h256(payload) {
            return Err(EventError::Invalid("payloadHash"));
        }
        let mut blank = self.clone();
        blank.event_id.clear();
        let expected = format!(
            "evt_{}",
            sha256_hex(canonical_bytes(&blank).map_err(serialization)?)
        );
        if self.event_id != expected {
            return Err(EventError::Invalid("eventId"));
        }
        Ok(())
    }

    pub fn payload_object(&self) -> Result<&Map<String, Value>, EventError> {
        self.payload
            .as_object()
            .ok_or(EventError::Invalid("payload"))
    }
}

fn validate_writer(writer: &Writer) -> Result<(), EventError> {
    if writer.process_id == 0 {
        return Err(EventError::Invalid("writer.processId"));
    }
    validate_timestamp_ms(writer.process_start_ms)
        .map_err(|_| EventError::Invalid("writer.processStartMs"))?;
    validate_prefixed_hex("hostId", &writer.host_id, "host_", 32)
        .map_err(|_| EventError::Invalid("writer.hostId"))?;
    validate_prefixed_hex("writerId", &writer.writer_id, "writer_", 64)
        .map_err(|_| EventError::Invalid("writer.writerId"))
}

fn validate_aggregate(event: &EventEnvelope) -> Result<(), EventError> {
    let object = event.payload_object()?;
    let expected = match event.aggregate.kind {
        AggregateKind::Request => event.request_id.as_deref(),
        AggregateKind::Session => string(object, "sessionId"),
        AggregateKind::Slot => string(object, "slotId"),
        AggregateKind::Allocator => Some("allocator"),
        AggregateKind::Claim => string(object, "claimId"),
        AggregateKind::Lease => string(object, "leaseId"),
        AggregateKind::RuntimeOwner => {
            string(object, "runtimeOwnerId").or_else(|| string(object, "newOwnerId"))
        }
        AggregateKind::ArtifactClaim => string(object, "artifactClaimId"),
        AggregateKind::Release => string(object, "releaseId"),
        AggregateKind::Qa => Some("qa"),
    };
    if expected != Some(event.aggregate.id.as_str()) {
        return Err(EventError::Invalid("aggregate.id"));
    }
    if aggregate_kind(event.event_type) != event.aggregate.kind {
        return Err(EventError::Invalid("aggregate.kind"));
    }
    Ok(())
}

pub fn aggregate_kind(event: EventType) -> AggregateKind {
    use EventType::*;
    match event {
        RequestAccepted
        | HostAttachmentsStaged
        | RootCaptureStarted
        | RootCaptureObserved
        | RootCaptureFailed
        | ModelSelectionStarted
        | ModelSelectionVerified
        | ModelSelectionFailed
        | SlotAttachmentsMaterialized
        | UploadStarted
        | UploadMismatchObserved
        | UploadCleared
        | UploadCompleted
        | UploadFailed
        | SendClickArmed
        | SendClicked
        | SendReconciled
        | SendUncertain
        | SendFailed
        | TurnStartConfirmed
        | RunningProjected
        | PollStarted
        | PollProgress
        | PollFailed
        | AnswerTerminal
        | TerminalPersisted
        | OutputPublished
        | OutputPublishFailed => AggregateKind::Request,
        SessionBindingEstablished
        | SessionRebindStarted
        | SessionRebound
        | SessionHydrationObserved
        | SessionHydrated
        | SessionOperationFailed => AggregateKind::Session,
        SlotHealthProbeStarted | SlotHealthObserved | SlotStandbyWritten | CooldownCleared => {
            AggregateKind::Slot
        }
        AllocationCandidateObserved | AllocationExhausted => AggregateKind::Allocator,
        RequestClaimGranted
        | RequestClaimRenewed
        | SessionOperationClaimGranted
        | SessionOperationClaimRenewed
        | SessionOperationClaimReleased
        | RequestClaimReleased => AggregateKind::Claim,
        SlotLeaseGranted | SlotLeaseRenewed | PersistedSessionLeaseGranted | SlotLeaseReleased => {
            AggregateKind::Lease
        }
        RuntimeOwnershipGranted
        | RuntimeOwnershipAdopted
        | RuntimeOwnershipRenewed
        | SessionRuntimeOwnershipGranted
        | SessionRuntimeOwnershipAdopted
        | RuntimeTakeoverProven
        | RuntimeOwnershipReleased => AggregateKind::RuntimeOwner,
        ArtifactClaimEstablished
        | ArtifactControlsAbsent
        | ArtifactControlsDiscovered
        | ArtifactDownloadAttemptConsumed
        | ArtifactRecoveryCandidateObserved
        | ArtifactDownloadCompleted
        | ArtifactClaimCompleted
        | ArtifactClaimFailed => AggregateKind::ArtifactClaim,
        ReleaseStarted
        | ReleaseEvidencePreserved
        | RuntimeStopStarted
        | RuntimeStopped
        | RuntimeStopFailed
        | RuntimeStopSkipped
        | ReleaseCleanupStarted
        | ReleaseCleanupFailed
        | ReleaseCleanupCommitted
        | ReleaseCooldownBlocked
        | ReleaseFinalized => AggregateKind::Release,
        SnapshotPublished | SnapshotRejected | QaMatrixRecorded | QaRepeatRecorded
        | QaCountersReset => AggregateKind::Qa,
    }
}

fn validate_payload_keys(event: EventType, object: &Map<String, Value>) -> Result<(), EventError> {
    let expected = payload_keys(event).iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    (actual == expected)
        .then_some(())
        .ok_or(EventError::Invalid("payload fields"))
}

fn payload_keys(event: EventType) -> &'static [&'static str] {
    use EventType::*;
    match event {
        RequestAccepted => &[
            "requestId",
            "kind",
            "promptSha256",
            "promptSizeBytes",
            "attachmentCount",
            "artifactExpectation",
            "acceptedAtMs",
        ],
        RequestClaimGranted => &[
            "claimId",
            "requestId",
            "claimGeneration",
            "ttlMs",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        RequestClaimRenewed | SessionOperationClaimRenewed => &[
            "claimId",
            "claimGeneration",
            "renewalRevision",
            "renewedAtMs",
            "renewAtMs",
            "expiresAtMs",
        ],
        HostAttachmentsStaged => &["requestId", "attachmentSet", "stagedAtMs"],
        AllocationCandidateObserved => &[
            "requestId",
            "scanOrdinal",
            "cohort",
            "slotId",
            "cohortCursorBefore",
            "withinCursorBefore",
            "decision",
            "skipReason",
            "observedAtMs",
        ],
        AllocationExhausted => &["requestId", "scanOrdinalCount", "observedAtMs"],
        SlotLeaseGranted => &[
            "leaseId",
            "claimId",
            "slotId",
            "cohort",
            "cohortCursorBefore",
            "withinCursorBefore",
            "cohortCursorAfter",
            "withinCursorAfter",
            "leaseGeneration",
            "reason",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        SlotLeaseRenewed => &[
            "leaseId",
            "leaseGeneration",
            "renewalRevision",
            "renewedAtMs",
            "renewAtMs",
            "expiresAtMs",
        ],
        RuntimeOwnershipGranted => &[
            "runtimeOwnerId",
            "slotId",
            "leaseId",
            "ownerGeneration",
            "runtimeIncarnationId",
            "dockerStatus",
            "startReceipt",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        RuntimeOwnershipAdopted => &[
            "runtimeOwnerId",
            "slotId",
            "leaseId",
            "ownerGeneration",
            "runtimeIncarnationId",
            "dockerStatus",
            "adoptionProof",
            "adoptedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        RuntimeOwnershipRenewed => &[
            "runtimeOwnerId",
            "ownerGeneration",
            "renewalRevision",
            "renewedAtMs",
            "renewAtMs",
            "expiresAtMs",
        ],
        SlotHealthProbeStarted => &[
            "slotId",
            "probeId",
            "dockerStatus",
            "deadlineMs",
            "retryIndex",
            "startedAtMs",
        ],
        SlotHealthObserved => &[
            "slotId",
            "probeId",
            "healthStatus",
            "dockerStatus",
            "cooldownMs",
            "allocatable",
            "evidenceRefs",
            "observedAtMs",
        ],
        RootCaptureStarted => &["requestId", "captureOperationId", "slotId", "startedAtMs"],
        RootCaptureObserved => &[
            "requestId",
            "captureOperationId",
            "rootBindingCandidate",
            "bindingId",
            "bindingGeneration",
            "pageBinding",
            "observedAtMs",
        ],
        RootCaptureFailed => &[
            "requestId",
            "captureOperationId",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        ModelSelectionStarted => &[
            "requestId",
            "modelOperationId",
            "requestedModel",
            "requestedEffort",
            "startedAtMs",
        ],
        ModelSelectionVerified => &[
            "requestId",
            "modelOperationId",
            "modelProof",
            "effortProof",
            "failureProof",
            "verifiedAtMs",
        ],
        ModelSelectionFailed => &[
            "requestId",
            "modelOperationId",
            "modelProof",
            "effortProof",
            "failureProof",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        SlotAttachmentsMaterialized => &[
            "requestId",
            "slotId",
            "attachmentSetSha256",
            "containerMountRoot",
            "materializedFiles",
            "materializedAtMs",
        ],
        UploadStarted => &[
            "requestId",
            "uploadAttemptId",
            "retryIndex",
            "expectedSetSha256",
            "expectedBindingHash",
            "startedAtMs",
        ],
        UploadMismatchObserved => &[
            "requestId",
            "uploadAttemptId",
            "uploadProof",
            "reason",
            "observedAtMs",
        ],
        UploadCleared => &[
            "requestId",
            "uploadAttemptId",
            "clearAttemptId",
            "clearedChips",
            "providerReceipt",
            "clearedAtMs",
        ],
        UploadCompleted => &[
            "requestId",
            "uploadAttemptId",
            "retryIndex",
            "uploadProof",
            "providerReceipt",
            "completedAtMs",
        ],
        UploadFailed => &[
            "requestId",
            "uploadAttemptId",
            "retryIndex",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        SendClickArmed => &[
            "requestId",
            "sendAttemptId",
            "uploadAttemptId",
            "expectedBindingHash",
            "promptSha256",
            "preClickReceiptPath",
            "postClickReceiptPath",
            "reconcileReceiptPath",
            "clickBudget",
            "armedAtMs",
        ],
        SendClicked => &[
            "requestId",
            "sendAttemptId",
            "preClickReceipt",
            "postClickReceipt",
            "physicalClickCount",
            "clickedAtMs",
        ],
        SendReconciled => &[
            "requestId",
            "sendAttemptId",
            "preClickReceipt",
            "reconciledReceipt",
            "physicalClickCount",
            "reconciledAtMs",
        ],
        SendUncertain => &["requestId", "sendAttemptId", "reason", "blockedAtMs"],
        SendFailed => &[
            "requestId",
            "sendAttemptId",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        TurnStartConfirmed => &[
            "requestId",
            "sessionId",
            "conversationUrl",
            "userTurnId",
            "assistantTurnId",
            "confirmedAtMs",
        ],
        SessionBindingEstablished => &[
            "sessionId",
            "sessionBindingId",
            "conversationUrl",
            "slotId",
            "cohort",
            "pageBindingId",
            "pageBindingGeneration",
            "targetId",
            "pageIncarnationId",
            "runtimeOwnerId",
            "establishedAtMs",
        ],
        RunningProjected => &[
            "requestId",
            "sessionId",
            "sessionBindingId",
            "activeTurn",
            "projectedAtMs",
        ],
        SessionOperationClaimGranted => &[
            "claimId",
            "sessionId",
            "operationKind",
            "expectedSlotId",
            "expectedCohort",
            "expectedRuntimeOwnerGeneration",
            "requestId",
            "runId",
            "ttlMs",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        PersistedSessionLeaseGranted => &[
            "leaseId",
            "claimId",
            "slotId",
            "cohort",
            "leaseGeneration",
            "reason",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        SessionRuntimeOwnershipGranted => &[
            "runtimeOwnerId",
            "sessionId",
            "slotId",
            "leaseId",
            "ownerGeneration",
            "runtimeIncarnationId",
            "dockerStatus",
            "startReceipt",
            "grantedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        SessionRuntimeOwnershipAdopted => &[
            "runtimeOwnerId",
            "sessionId",
            "slotId",
            "leaseId",
            "ownerGeneration",
            "runtimeIncarnationId",
            "dockerStatus",
            "adoptionProof",
            "adoptedAtMs",
            "renewAtMs",
            "expiresAtMs",
            "fencingTokenSha256",
        ],
        SessionRebindStarted => &[
            "sessionId",
            "sessionOperationClaimId",
            "operationKind",
            "expectationSha256",
            "navigationAttemptLimit",
            "hydrationDeadlineMs",
            "startedAtMs",
        ],
        SessionRebound => &[
            "sessionId",
            "expectation",
            "observedEcho",
            "pageBindingGeneration",
            "providerReceipt",
            "reboundAtMs",
        ],
        SessionHydrationObserved => &[
            "sessionId",
            "hydrationObservation",
            "sequenceIndex",
            "remainingDeadlineMs",
            "observedAtMs",
        ],
        SessionHydrated => &[
            "sessionId",
            "observations",
            "terminalVisible",
            "activeGeneration",
            "contentUnavailable",
            "finalObservation",
            "hydratedAtMs",
        ],
        SessionOperationFailed => &[
            "sessionId",
            "sessionOperationClaimId",
            "operationKind",
            "stage",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        PollStarted => &[
            "requestId",
            "pollAttemptId",
            "sessionId",
            "pollTimeoutSeconds",
            "startedAtMs",
        ],
        PollProgress => &[
            "requestId",
            "pollAttemptId",
            "providerStatus",
            "activeGeneration",
            "sequenceIndex",
            "pollReceipt",
            "observedAtMs",
        ],
        PollFailed => &[
            "requestId",
            "pollAttemptId",
            "reason",
            "providerReceipt",
            "failedAtMs",
        ],
        AnswerTerminal => &[
            "requestId",
            "pollAttemptId",
            "sessionId",
            "answerPath",
            "answerSha256",
            "answerSizeBytes",
            "terminalAssistantTurnId",
            "pollReceipt",
            "terminalAtMs",
        ],
        ArtifactClaimEstablished => &[
            "artifactClaimId",
            "sessionId",
            "requestId",
            "expectation",
            "terminalAssistantTurnId",
            "establishedAtMs",
        ],
        ArtifactControlsAbsent => &[
            "artifactClaimId",
            "zeroControlProof",
            "providerReceipt",
            "observedAtMs",
        ],
        ArtifactControlsDiscovered => &[
            "artifactClaimId",
            "controls",
            "controlCount",
            "bottomProof",
            "providerReceipt",
            "discoveredAtMs",
        ],
        ArtifactDownloadAttemptConsumed => &[
            "artifactClaimId",
            "attemptId",
            "controlIndex",
            "controlId",
            "artifactBaseline",
            "providerReceiptPath",
            "hostSaveDirectory",
            "clickBudget",
            "attemptConsumedAtMs",
        ],
        ArtifactRecoveryCandidateObserved => &[
            "artifactClaimId",
            "attemptId",
            "candidateRelPath",
            "sizeBytes",
            "sha256",
            "stableObservations",
            "observedAtMs",
        ],
        ArtifactDownloadCompleted => &[
            "artifactClaimId",
            "attemptId",
            "controlIndex",
            "artifactId",
            "downloadReceipt",
            "completedAtMs",
        ],
        ArtifactClaimCompleted => &[
            "artifactClaimId",
            "result",
            "artifactCount",
            "manifestPath",
            "manifestSha256",
            "completedAtMs",
        ],
        ArtifactClaimFailed => &[
            "artifactClaimId",
            "reason",
            "failedControlIndex",
            "providerReceipt",
            "failedAtMs",
        ],
        TerminalPersisted => &[
            "requestId",
            "answerTerminalEventId",
            "artifactClaimEventIds",
            "outputPath",
            "persistedAtMs",
        ],
        OutputPublished => &["requestId", "outputPath", "outputSha256", "publishedAtMs"],
        OutputPublishFailed => &["requestId", "reason", "failedAtMs"],
        ReleaseStarted => &[
            "releaseId",
            "subjectKind",
            "subjectId",
            "reason",
            "startedAtMs",
        ],
        ReleaseEvidencePreserved => &[
            "releaseId",
            "evidenceManifestPath",
            "evidenceManifestSha256",
            "preservedAtMs",
        ],
        RuntimeTakeoverProven => &[
            "releaseId",
            "slotId",
            "priorOwnerId",
            "priorGeneration",
            "newOwnerId",
            "newGeneration",
            "deadOwnerProof",
            "provenAtMs",
        ],
        RuntimeStopStarted => &[
            "releaseId",
            "runtimeOwnerId",
            "ownerGeneration",
            "stopTimeoutMs",
            "startedAtMs",
        ],
        RuntimeStopped => &[
            "releaseId",
            "runtimeOwnerId",
            "ownerGeneration",
            "dockerStatus",
            "stopReceipt",
            "stoppedAtMs",
        ],
        RuntimeStopFailed => &[
            "releaseId",
            "runtimeOwnerId",
            "ownerGeneration",
            "dockerStatus",
            "failureReceipt",
            "reason",
            "failedAtMs",
        ],
        RuntimeStopSkipped => &[
            "releaseId",
            "runtimeOwnerId",
            "reason",
            "proofAttempt",
            "skippedAtMs",
        ],
        ReleaseCleanupStarted => &["releaseId", "startedAtMs"],
        ReleaseCleanupFailed => &["releaseId", "reason", "failedAtMs"],
        SessionOperationClaimReleased | RequestClaimReleased => {
            &["claimId", "claimGeneration", "releaseId", "releasedAtMs"]
        }
        SlotLeaseReleased => &["leaseId", "leaseGeneration", "releaseId", "releasedAtMs"],
        RuntimeOwnershipReleased => &[
            "runtimeOwnerId",
            "ownerGeneration",
            "releaseId",
            "runtimeOutcome",
            "releasedAtMs",
        ],
        ReleaseCleanupCommitted => &[
            "releaseId",
            "requestClaimReleaseMode",
            "sessionClaimReleaseMode",
            "leaseReleaseMode",
            "ownerReleaseMode",
            "committedAtMs",
        ],
        SlotStandbyWritten => &[
            "slotId",
            "releaseId",
            "allocatable",
            "cooldownUntilMs",
            "writtenAtMs",
        ],
        ReleaseCooldownBlocked => &["releaseId", "slotId", "cooldownUntilMs", "blockedAtMs"],
        CooldownCleared => &["slotId", "clearedAtMs"],
        ReleaseFinalized => &["releaseId", "finalStatus", "allocatable", "finalizedAtMs"],
        SnapshotPublished => &[
            "projectionName",
            "snapshotPath",
            "lastEventId",
            "projectionDigest",
            "snapshotSha256",
            "publishedAtMs",
        ],
        SnapshotRejected => &[
            "projectionName",
            "snapshotPath",
            "reason",
            "expectedDigest",
            "observedDigest",
            "rejectedAtMs",
        ],
        QaMatrixRecorded => &[
            "qaRunId",
            "matrixIteration",
            "sourceFingerprint",
            "evidenceDigest",
            "casesPassed",
            "casesTotal",
            "recordedAtMs",
        ],
        QaRepeatRecorded => &[
            "qaRunId",
            "caseId",
            "repetitionIndex",
            "sourceFingerprint",
            "passed",
            "recordedAtMs",
        ],
        QaCountersReset => &[
            "qaRunId",
            "reason",
            "sourceFingerprint",
            "scope",
            "caseId",
            "resetAtMs",
        ],
    }
}

fn validate_payload_semantics(event: &EventEnvelope) -> Result<(), EventError> {
    use EventType::*;
    let payload = Payload(event.payload_object()?);
    match event.event_type {
        RequestAccepted => {
            payload.id("requestId", validate_request_id)?;
            payload.one_of("kind", &["pro", "xhigh"])?;
            payload.id("promptSha256", validate_h256)?;
            payload.bytes("promptSizeBytes")?;
            payload.integer("attachmentCount", 0, 64)?;
            payload.expectation("artifactExpectation")?;
            payload.timestamp("acceptedAtMs")?;
        }
        RequestClaimGranted => {
            payload.id("claimId", validate_claim_id)?;
            payload.id("requestId", validate_request_id)?;
            payload.literal_u64("claimGeneration", 1)?;
            payload.literal_u64("ttlMs", 300_000)?;
            payload.cas_times("grantedAtMs")?;
            payload.id("fencingTokenSha256", validate_h256)?;
        }
        RequestClaimRenewed | SessionOperationClaimRenewed => {
            payload.id("claimId", validate_claim_id)?;
            payload.integer("claimGeneration", 1, 65_535)?;
            payload.integer("renewalRevision", 2, 65_535)?;
            payload.cas_times("renewedAtMs")?;
        }
        HostAttachmentsStaged => {
            let request_id = payload.id("requestId", validate_request_id)?;
            let run_id = event
                .run_id
                .as_deref()
                .ok_or(EventError::Invalid("HostAttachmentsStaged runId"))?;
            payload
                .parse::<AttachmentSet>("attachmentSet")?
                .validate_for(request_id, run_id)
                .map_err(|_| EventError::Invalid("attachmentSet"))?;
            payload.timestamp("stagedAtMs")?;
        }
        AllocationCandidateObserved => {
            payload.id("requestId", validate_request_id)?;
            payload.integer("scanOrdinal", 0, 9)?;
            let cohort = payload.id("cohort", validate_cohort)?;
            let slot = payload.id("slotId", validate_slot_id)?;
            if crate::allocator::cohort_of(slot) != Some(cohort) {
                return Err(EventError::Invalid("allocation cohort/slot"));
            }
            payload.integer("cohortCursorBefore", 0, 2)?;
            payload.integer("withinCursorBefore", 0, 3)?;
            match payload.one_of("decision", &["grantable", "skip"])? {
                "grantable" => payload.null("skipReason")?,
                "skip" => {
                    payload.one_of(
                        "skipReason",
                        &[
                            "leased",
                            "claim_active",
                            "cooldown",
                            "runtime_owned",
                            "health_blocked",
                            "state_invalid",
                        ],
                    )?;
                }
                _ => unreachable!(),
            }
            payload.timestamp("observedAtMs")?;
        }
        AllocationExhausted => {
            payload.id("requestId", validate_request_id)?;
            payload.literal_u64("scanOrdinalCount", 10)?;
            payload.timestamp("observedAtMs")?;
        }
        SlotLeaseGranted => validate_lease_grant(&payload, false)?,
        PersistedSessionLeaseGranted => validate_lease_grant(&payload, true)?,
        SlotLeaseRenewed => {
            payload.id("leaseId", validate_lease_id)?;
            payload.integer("leaseGeneration", 1, 65_535)?;
            payload.integer("renewalRevision", 2, 65_535)?;
            payload.cas_times("renewedAtMs")?;
        }
        RuntimeOwnershipGranted | SessionRuntimeOwnershipGranted => validate_owner_grant(
            &payload,
            false,
            event.event_type == SessionRuntimeOwnershipGranted,
        )?,
        RuntimeOwnershipAdopted | SessionRuntimeOwnershipAdopted => validate_owner_grant(
            &payload,
            true,
            event.event_type == SessionRuntimeOwnershipAdopted,
        )?,
        RuntimeOwnershipRenewed => {
            payload.id("runtimeOwnerId", validate_owner_id)?;
            payload.integer("ownerGeneration", 1, 65_535)?;
            payload.integer("renewalRevision", 2, 65_535)?;
            payload.cas_times("renewedAtMs")?;
        }
        SlotHealthProbeStarted => {
            payload.id("slotId", validate_slot_id)?;
            payload.id("probeId", validate_operation_id)?;
            payload.docker("dockerStatus")?;
            payload.literal_u64("deadlineMs", 15_000)?;
            payload.integer("retryIndex", 0, 1)?;
            payload.timestamp("startedAtMs")?;
        }
        SlotHealthObserved => {
            payload.id("slotId", validate_slot_id)?;
            payload.id("probeId", validate_operation_id)?;
            payload.health("healthStatus")?;
            payload.docker("dockerStatus")?;
            payload.duration("cooldownMs")?;
            payload.boolean("allocatable")?;
            payload.evidence("evidenceRefs", 0, 4)?;
            payload.timestamp("observedAtMs")?;
        }
        RootCaptureStarted => {
            payload.id("requestId", validate_request_id)?;
            payload.id("captureOperationId", validate_operation_id)?;
            payload.id("slotId", validate_slot_id)?;
            payload.timestamp("startedAtMs")?;
        }
        RootCaptureObserved => {
            payload.id("requestId", validate_request_id)?;
            payload.id("captureOperationId", validate_operation_id)?;
            let root = payload.parse::<RootBindingCandidate>("rootBindingCandidate")?;
            root.validate()
                .map_err(|_| EventError::Invalid("rootBindingCandidate"))?;
            let binding_id = payload.id("bindingId", validate_binding_id)?;
            payload.literal_u64("bindingGeneration", 1)?;
            let page = payload.parse::<PageBindingEcho>("pageBinding")?;
            page.validate()
                .map_err(|_| EventError::Invalid("pageBinding"))?;
            if page.binding_id != binding_id || page.binding_generation != 1 {
                return Err(EventError::Invalid("root page binding"));
            }
            payload.timestamp("observedAtMs")?;
        }
        RootCaptureFailed => {
            payload.id("requestId", validate_request_id)?;
            payload.id("captureOperationId", validate_operation_id)?;
            payload.one_of(
                "reason",
                &[
                    "capture.ambiguous",
                    "capture.timeout",
                    "contract.invalid_provider_envelope",
                    "binding.mismatch",
                ],
            )?;
            payload.optional_evidence("providerReceipt")?;
            payload.timestamp("failedAtMs")?;
        }
        ModelSelectionStarted => {
            payload.id("requestId", validate_request_id)?;
            payload.id("modelOperationId", validate_operation_id)?;
            let model = payload.parse::<Model>("requestedModel")?;
            let effort = payload.parse::<Effort>("requestedEffort")?;
            super::browser::validate_model_tuple(&model, &effort)
                .map_err(|_| EventError::Invalid("model tuple"))?;
            payload.timestamp("startedAtMs")?;
        }
        ModelSelectionVerified => {
            payload.id("requestId", validate_request_id)?;
            payload.id("modelOperationId", validate_operation_id)?;
            let model = payload.parse::<ModelProof>("modelProof")?;
            let effort = payload.parse::<EffortProof>("effortProof")?;
            model
                .validate()
                .map_err(|_| EventError::Invalid("modelProof"))?;
            effort
                .validate()
                .map_err(|_| EventError::Invalid("effortProof"))?;
            super::browser::validate_model_tuple(&model.requested, &effort.requested)
                .map_err(|_| EventError::Invalid("model proof tuple"))?;
            payload.null("failureProof")?;
            payload.timestamp("verifiedAtMs")?;
        }
        ModelSelectionFailed => validate_model_failed(&payload)?,
        SlotAttachmentsMaterialized => validate_materialized(&payload)?,
        UploadStarted => {
            payload.id("requestId", validate_request_id)?;
            payload.id("uploadAttemptId", validate_operation_id)?;
            payload.integer("retryIndex", 0, 1)?;
            payload.id("expectedSetSha256", validate_h256)?;
            payload.id("expectedBindingHash", validate_h256)?;
            payload.timestamp("startedAtMs")?;
        }
        UploadMismatchObserved => {
            payload.id("requestId", validate_request_id)?;
            let attempt = payload.id("uploadAttemptId", validate_operation_id)?;
            let proof = payload.parse::<UploadProof>("uploadProof")?;
            proof
                .validate()
                .map_err(|_| EventError::Invalid("uploadProof"))?;
            if proof.upload_attempt_id != attempt || proof.retry_index != 0 {
                return Err(EventError::Invalid("upload mismatch binding"));
            }
            payload.literal("reason", "upload.stale_chip_mismatch")?;
            payload.timestamp("observedAtMs")?;
        }
        UploadCleared => validate_upload_cleared(&payload)?,
        UploadCompleted => {
            payload.id("requestId", validate_request_id)?;
            let attempt = payload.id("uploadAttemptId", validate_operation_id)?;
            let retry = payload.integer("retryIndex", 0, 1)? as u8;
            let proof = payload.parse::<UploadProof>("uploadProof")?;
            proof
                .validate()
                .map_err(|_| EventError::Invalid("uploadProof"))?;
            if proof.upload_attempt_id != attempt
                || proof.retry_index != retry
                || !proof.stale_chips.is_empty()
                || !proof.all_expected_complete
            {
                return Err(EventError::Invalid("UploadCompleted proof"));
            }
            payload.evidence_ref("providerReceipt")?;
            payload.timestamp("completedAtMs")?;
        }
        UploadFailed => {
            payload.id("requestId", validate_request_id)?;
            payload.id("uploadAttemptId", validate_operation_id)?;
            payload.integer("retryIndex", 0, 1)?;
            payload.one_of(
                "reason",
                &[
                    "upload.stale_chip_uncleared",
                    "upload.incomplete",
                    "upload.chip_removal_failed",
                    "contract.invalid_provider_envelope",
                    "binding.mismatch",
                ],
            )?;
            payload.optional_evidence("providerReceipt")?;
            payload.timestamp("failedAtMs")?;
        }
        SendClickArmed => {
            payload.id("requestId", validate_request_id)?;
            payload.id("sendAttemptId", validate_operation_id)?;
            payload.id("uploadAttemptId", validate_operation_id)?;
            payload.id("expectedBindingHash", validate_h256)?;
            payload.id("promptSha256", validate_h256)?;
            for key in [
                "preClickReceiptPath",
                "postClickReceiptPath",
                "reconcileReceiptPath",
            ] {
                payload.id(key, validate_safe_rel_path)?;
            }
            payload.literal_u64("clickBudget", 1)?;
            payload.timestamp("armedAtMs")?;
        }
        SendClicked | SendReconciled => validate_send_terminal(&payload, event.event_type)?,
        SendUncertain => {
            payload.id("requestId", validate_request_id)?;
            payload.id("sendAttemptId", validate_operation_id)?;
            payload.literal("reason", "send.turn_not_proven")?;
            payload.timestamp("blockedAtMs")?;
        }
        SendFailed => {
            payload.id("requestId", validate_request_id)?;
            payload.id("sendAttemptId", validate_operation_id)?;
            payload.one_of(
                "reason",
                &[
                    "contract.invalid_provider_envelope",
                    "binding.mismatch",
                    "send.click_timeout",
                ],
            )?;
            payload.optional_evidence("providerReceipt")?;
            payload.timestamp("failedAtMs")?;
        }
        TurnStartConfirmed => {
            payload.id("requestId", validate_request_id)?;
            let session = payload.id("sessionId", validate_session_id)?;
            validate_conversation_url(payload.text("conversationUrl")?, session)
                .map_err(|_| EventError::Invalid("conversationUrl"))?;
            payload.id("userTurnId", validate_turn_id)?;
            payload.id("assistantTurnId", validate_turn_id)?;
            payload.timestamp("confirmedAtMs")?;
        }
        SessionBindingEstablished => validate_session_binding(&payload)?,
        RunningProjected => {
            payload.id("requestId", validate_request_id)?;
            payload.id("sessionId", validate_session_id)?;
            payload.id("sessionBindingId", validate_binding_id)?;
            if !payload.boolean("activeTurn")? {
                return Err(EventError::Invalid("activeTurn"));
            }
            payload.timestamp("projectedAtMs")?;
        }
        SessionOperationClaimGranted => validate_session_claim(&payload, event)?,
        SessionRebindStarted => {
            payload.id("sessionId", validate_session_id)?;
            payload.optional_id("sessionOperationClaimId", validate_claim_id)?;
            payload.operation("operationKind", &["resume", "show", "download", "poll"])?;
            payload.id("expectationSha256", validate_h256)?;
            payload.literal_u64("navigationAttemptLimit", 2)?;
            payload.literal_u64("hydrationDeadlineMs", 90_000)?;
            payload.timestamp("startedAtMs")?;
        }
        SessionRebound => validate_session_rebound(&payload)?,
        SessionHydrationObserved => validate_hydration_observed(&payload)?,
        SessionHydrated => validate_session_hydrated(&payload)?,
        SessionOperationFailed => {
            payload.id("sessionId", validate_session_id)?;
            payload.optional_id("sessionOperationClaimId", validate_claim_id)?;
            payload.operation("operationKind", &["resume", "show", "download", "poll"])?;
            payload.one_of(
                "stage",
                &["lease", "runtime", "rebind", "hydration", "content"],
            )?;
            payload.session_failure("reason")?;
            payload.optional_evidence("providerReceipt")?;
            payload.timestamp("failedAtMs")?;
        }
        PollStarted => {
            payload.id("requestId", validate_request_id)?;
            payload.id("pollAttemptId", validate_operation_id)?;
            payload.id("sessionId", validate_session_id)?;
            payload.integer("pollTimeoutSeconds", 1, 10_800)?;
            payload.timestamp("startedAtMs")?;
        }
        PollProgress => {
            payload.id("requestId", validate_request_id)?;
            payload.id("pollAttemptId", validate_operation_id)?;
            payload.non_empty("providerStatus")?;
            payload.boolean("activeGeneration")?;
            payload.integer("sequenceIndex", 0, 65_535)?;
            payload.evidence_ref("pollReceipt")?;
            payload.timestamp("observedAtMs")?;
        }
        PollFailed => {
            payload.id("requestId", validate_request_id)?;
            payload.id("pollAttemptId", validate_operation_id)?;
            payload.poll_failure("reason")?;
            payload.optional_evidence("providerReceipt")?;
            payload.timestamp("failedAtMs")?;
        }
        AnswerTerminal => validate_answer_terminal(&payload)?,
        ArtifactClaimEstablished => {
            payload.id("artifactClaimId", validate_artifact_claim_id)?;
            payload.id("sessionId", validate_session_id)?;
            payload.optional_id("requestId", validate_request_id)?;
            payload.expectation("expectation")?;
            payload.id("terminalAssistantTurnId", validate_turn_id)?;
            payload.timestamp("establishedAtMs")?;
        }
        ArtifactControlsAbsent => validate_controls_absent(&payload)?,
        ArtifactControlsDiscovered => validate_controls_discovered(&payload)?,
        ArtifactDownloadAttemptConsumed => validate_artifact_attempt(&payload)?,
        ArtifactRecoveryCandidateObserved => {
            payload.id("artifactClaimId", validate_artifact_claim_id)?;
            payload.id("attemptId", validate_operation_id)?;
            payload.id("candidateRelPath", validate_safe_rel_path)?;
            payload.bytes("sizeBytes")?;
            payload.id("sha256", validate_h256)?;
            payload.literal_u64("stableObservations", 2)?;
            payload.timestamp("observedAtMs")?;
        }
        ArtifactDownloadCompleted => validate_artifact_download(&payload)?,
        ArtifactClaimCompleted => validate_artifact_completed(&payload)?,
        ArtifactClaimFailed => validate_artifact_failed(&payload)?,
        TerminalPersisted => {
            payload.id("requestId", validate_request_id)?;
            payload.id("answerTerminalEventId", validate_event_id)?;
            payload.id_array("artifactClaimEventIds", validate_event_id, 0, 16)?;
            payload.id("outputPath", validate_safe_rel_path)?;
            payload.timestamp("persistedAtMs")?;
        }
        OutputPublished => {
            payload.id("requestId", validate_request_id)?;
            payload.id("outputPath", validate_safe_rel_path)?;
            payload.id("outputSha256", validate_h256)?;
            payload.timestamp("publishedAtMs")?;
        }
        OutputPublishFailed => {
            payload.id("requestId", validate_request_id)?;
            payload.non_empty("reason")?;
            payload.timestamp("failedAtMs")?;
        }
        ReleaseStarted => validate_release_started(&payload)?,
        ReleaseEvidencePreserved => {
            payload.id("releaseId", validate_release_id)?;
            payload.id("evidenceManifestPath", validate_safe_rel_path)?;
            payload.id("evidenceManifestSha256", validate_h256)?;
            payload.timestamp("preservedAtMs")?;
        }
        RuntimeTakeoverProven => validate_takeover(&payload)?,
        RuntimeStopStarted => {
            payload.id("releaseId", validate_release_id)?;
            payload.id("runtimeOwnerId", validate_owner_id)?;
            payload.integer("ownerGeneration", 1, 65_535)?;
            payload.duration("stopTimeoutMs")?;
            payload.timestamp("startedAtMs")?;
        }
        RuntimeStopped => {
            validate_runtime_stop_common(&payload, "stoppedAtMs")?;
            payload.docker("dockerStatus")?;
            payload.evidence_ref("stopReceipt")?;
        }
        RuntimeStopFailed => {
            validate_runtime_stop_common(&payload, "failedAtMs")?;
            payload.docker("dockerStatus")?;
            payload.evidence_ref("failureReceipt")?;
            payload.non_empty("reason")?;
        }
        RuntimeStopSkipped => validate_stop_skipped(&payload)?,
        ReleaseCleanupStarted => {
            payload.id("releaseId", validate_release_id)?;
            payload.timestamp("startedAtMs")?;
        }
        ReleaseCleanupFailed => {
            payload.id("releaseId", validate_release_id)?;
            payload.non_empty("reason")?;
            payload.timestamp("failedAtMs")?;
        }
        SessionOperationClaimReleased | RequestClaimReleased => {
            payload.id("claimId", validate_claim_id)?;
            payload.integer("claimGeneration", 1, 65_535)?;
            payload.id("releaseId", validate_release_id)?;
            payload.timestamp("releasedAtMs")?;
        }
        SlotLeaseReleased => {
            payload.id("leaseId", validate_lease_id)?;
            payload.integer("leaseGeneration", 1, 65_535)?;
            payload.id("releaseId", validate_release_id)?;
            payload.timestamp("releasedAtMs")?;
        }
        RuntimeOwnershipReleased => {
            payload.id("runtimeOwnerId", validate_owner_id)?;
            payload.integer("ownerGeneration", 1, 65_535)?;
            payload.id("releaseId", validate_release_id)?;
            payload.one_of("runtimeOutcome", &["stopped", "skipped", "failed"])?;
            payload.timestamp("releasedAtMs")?;
        }
        ReleaseCleanupCommitted => {
            payload.id("releaseId", validate_release_id)?;
            for key in [
                "requestClaimReleaseMode",
                "sessionClaimReleaseMode",
                "leaseReleaseMode",
                "ownerReleaseMode",
            ] {
                payload.one_of(key, &["released", "not_applicable"])?;
            }
            payload.timestamp("committedAtMs")?;
        }
        SlotStandbyWritten => {
            payload.id("slotId", validate_slot_id)?;
            payload.id("releaseId", validate_release_id)?;
            let allocatable = payload.boolean("allocatable")?;
            let cooldown = payload.optional_timestamp("cooldownUntilMs")?;
            if allocatable && cooldown.is_some() {
                return Err(EventError::Invalid("standby cooldown"));
            }
            payload.timestamp("writtenAtMs")?;
        }
        ReleaseCooldownBlocked => {
            payload.id("releaseId", validate_release_id)?;
            payload.id("slotId", validate_slot_id)?;
            let until = payload.timestamp("cooldownUntilMs")?;
            let blocked = payload.timestamp("blockedAtMs")?;
            if until <= blocked {
                return Err(EventError::Invalid("cooldown interval"));
            }
        }
        CooldownCleared => {
            payload.id("slotId", validate_slot_id)?;
            payload.timestamp("clearedAtMs")?;
        }
        ReleaseFinalized => {
            payload.id("releaseId", validate_release_id)?;
            let status = payload.one_of(
                "finalStatus",
                &[
                    "allocatable",
                    "cooldown_blocked",
                    "cleanup_failed",
                    "stop_skipped_owner_alive",
                    "resources_released_no_slot",
                ],
            )?;
            let allocatable = payload.boolean("allocatable")?;
            if allocatable != (status == "allocatable") {
                return Err(EventError::Invalid("ReleaseFinalized allocatable"));
            }
            payload.timestamp("finalizedAtMs")?;
        }
        SnapshotPublished => {
            payload.non_empty("projectionName")?;
            payload.id("snapshotPath", validate_safe_rel_path)?;
            payload.id("lastEventId", validate_event_id)?;
            payload.id("projectionDigest", validate_h256)?;
            payload.id("snapshotSha256", validate_h256)?;
            payload.timestamp("publishedAtMs")?;
        }
        SnapshotRejected => {
            payload.non_empty("projectionName")?;
            payload.id("snapshotPath", validate_safe_rel_path)?;
            payload.non_empty("reason")?;
            payload.optional_id("expectedDigest", validate_h256)?;
            payload.optional_id("observedDigest", validate_h256)?;
            payload.timestamp("rejectedAtMs")?;
        }
        QaMatrixRecorded => {
            payload.id("qaRunId", validate_operation_id)?;
            payload.integer("matrixIteration", 1, 3)?;
            payload.id("sourceFingerprint", validate_h256)?;
            payload.id("evidenceDigest", validate_h256)?;
            let passed = payload.integer("casesPassed", 0, 64)?;
            let total = payload.integer("casesTotal", 1, 64)?;
            if passed > total {
                return Err(EventError::Invalid("QA matrix counts"));
            }
            payload.timestamp("recordedAtMs")?;
        }
        QaRepeatRecorded => {
            payload.id("qaRunId", validate_operation_id)?;
            payload.non_empty("caseId")?;
            payload.integer("repetitionIndex", 1, 10)?;
            payload.id("sourceFingerprint", validate_h256)?;
            payload.boolean("passed")?;
            payload.timestamp("recordedAtMs")?;
        }
        QaCountersReset => {
            payload.id("qaRunId", validate_operation_id)?;
            payload.non_empty("reason")?;
            payload.id("sourceFingerprint", validate_h256)?;
            match payload.one_of("scope", &["all", "case"])? {
                "all" => payload.null("caseId")?,
                "case" => {
                    payload.non_empty("caseId")?;
                }
                _ => unreachable!(),
            }
            payload.timestamp("resetAtMs")?;
        }
    }
    Ok(())
}

fn validate_lease_grant(payload: &Payload<'_>, persisted: bool) -> Result<(), EventError> {
    payload.id("leaseId", validate_lease_id)?;
    payload.id("claimId", validate_claim_id)?;
    let slot = payload.id("slotId", validate_slot_id)?;
    let cohort = payload.id("cohort", validate_cohort)?;
    if crate::allocator::cohort_of(slot) != Some(cohort) {
        return Err(EventError::Invalid("lease cohort"));
    }
    if persisted {
        payload.literal_u64("leaseGeneration", 1)?;
        payload.literal("reason", "persisted_session")?;
    } else {
        payload.integer("cohortCursorBefore", 0, 2)?;
        payload.integer("withinCursorBefore", 0, 3)?;
        payload.integer("cohortCursorAfter", 0, 2)?;
        payload.integer("withinCursorAfter", 0, 3)?;
        payload.literal_u64("leaseGeneration", 1)?;
        payload.one_of("reason", &["fresh_send", "persisted_session"])?;
    }
    payload.cas_times("grantedAtMs")?;
    payload.id("fencingTokenSha256", validate_h256)?;
    Ok(())
}

fn validate_owner_grant(
    payload: &Payload<'_>,
    adopted: bool,
    session_scoped: bool,
) -> Result<(), EventError> {
    payload.id("runtimeOwnerId", validate_owner_id)?;
    if session_scoped {
        payload.id("sessionId", validate_session_id)?;
    }
    payload.id("slotId", validate_slot_id)?;
    payload.id("leaseId", validate_lease_id)?;
    payload.integer("ownerGeneration", 1, 65_535)?;
    payload.id(
        "runtimeIncarnationId",
        super::ids::validate_runtime_incarnation_id,
    )?;
    payload.docker("dockerStatus")?;
    if adopted {
        let proof = payload.parse::<AdoptionProof>("adoptionProof")?;
        validate_adoption_shape(&proof)?;
        payload.cas_times("adoptedAtMs")?;
    } else {
        payload.evidence_ref("startReceipt")?;
        payload.cas_times("grantedAtMs")?;
    }
    payload.id("fencingTokenSha256", validate_h256)?;
    Ok(())
}

fn validate_model_failed(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("requestId", validate_request_id)?;
    payload.id("modelOperationId", validate_operation_id)?;
    payload.null("modelProof")?;
    payload.null("effortProof")?;
    let reason = payload.non_empty("reason")?;
    let proof = payload.optional_parse::<FailureProof>("failureProof")?;
    let receipt = payload.optional_parse::<EvidenceRef>("providerReceipt")?;
    if let Some(ref receipt) = receipt {
        receipt
            .validate()
            .map_err(|_| EventError::Invalid("providerReceipt"))?;
    }
    let model_reason = matches!(
        reason,
        "picker.model_absent"
            | "picker.effort_absent"
            | "picker.control_drift"
            | "picker.selection_timeout"
            | "picker.reverify_mismatch"
            | "capture.ambiguous"
    );
    match (model_reason, proof) {
        (true, Some(proof)) if proof.reason == reason => proof
            .validate()
            .map_err(|_| EventError::Invalid("failureProof"))?,
        (false, None)
            if matches!(
                reason,
                "contract.invalid_provider_envelope" | "binding.mismatch" | "provider.schema_drift"
            ) => {}
        _ => return Err(EventError::Invalid("ModelSelectionFailed proof")),
    }
    if reason == "provider.schema_drift" && receipt.is_none() {
        return Err(EventError::Invalid("schema drift receipt"));
    }
    payload.timestamp("failedAtMs")?;
    Ok(())
}

fn validate_materialized(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("requestId", validate_request_id)?;
    payload.id("slotId", validate_slot_id)?;
    payload.id("attachmentSetSha256", validate_h256)?;
    payload.id("containerMountRoot", validate_safe_rel_path)?;
    let files = payload.array("materializedFiles", 0, 64)?;
    for file in files {
        let object = closed_object(file, &["containerRelPath", "sha256", "sizeBytes"])?;
        Payload(object).id("containerRelPath", validate_safe_rel_path)?;
        Payload(object).id("sha256", validate_h256)?;
        Payload(object).bytes("sizeBytes")?;
    }
    payload.timestamp("materializedAtMs")?;
    Ok(())
}

fn validate_upload_cleared(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("requestId", validate_request_id)?;
    payload.id("uploadAttemptId", validate_operation_id)?;
    payload.id("clearAttemptId", validate_operation_id)?;
    for chip in payload.array("clearedChips", 1, 64)? {
        let object = closed_object(chip, &["chipStableKey", "digest", "cleared"])?;
        let chip = Payload(object);
        chip.id("chipStableKey", validate_h256)?;
        chip.optional_id("digest", validate_h256)?;
        if !chip.boolean("cleared")? {
            return Err(EventError::Invalid("cleared chip"));
        }
    }
    payload.evidence_ref("providerReceipt")?;
    payload.timestamp("clearedAtMs")?;
    Ok(())
}

fn validate_send_terminal(payload: &Payload<'_>, kind: EventType) -> Result<(), EventError> {
    payload.id("requestId", validate_request_id)?;
    let attempt = payload.id("sendAttemptId", validate_operation_id)?;
    let pre = payload.parse::<SendReceipt>("preClickReceipt")?;
    let (terminal, count, time) = if kind == EventType::SendClicked {
        (
            payload.parse::<SendReceipt>("postClickReceipt")?,
            1,
            "clickedAtMs",
        )
    } else {
        (
            payload.parse::<SendReceipt>("reconciledReceipt")?,
            0,
            "reconciledAtMs",
        )
    };
    validate_receipt_pair(&pre, &terminal, &pre.page_binding)
        .map_err(|_| EventError::Invalid("send receipt pair"))?;
    if pre.send_attempt_id != attempt
        || payload.integer("physicalClickCount", count, count)? != count
    {
        return Err(EventError::Invalid("send receipt binding"));
    }
    payload.timestamp(time)?;
    Ok(())
}

fn validate_session_binding(payload: &Payload<'_>) -> Result<(), EventError> {
    let session = payload.id("sessionId", validate_session_id)?;
    payload.id("sessionBindingId", validate_binding_id)?;
    validate_conversation_url(payload.text("conversationUrl")?, session)
        .map_err(|_| EventError::Invalid("conversationUrl"))?;
    let slot = payload.id("slotId", validate_slot_id)?;
    let cohort = payload.id("cohort", validate_cohort)?;
    if crate::allocator::cohort_of(slot) != Some(cohort) {
        return Err(EventError::Invalid("session cohort"));
    }
    payload.id("pageBindingId", validate_binding_id)?;
    payload.literal_u64("pageBindingGeneration", 1)?;
    payload.id("targetId", validate_target_id)?;
    payload.id("pageIncarnationId", validate_page_incarnation_id)?;
    payload.id("runtimeOwnerId", validate_owner_id)?;
    payload.timestamp("establishedAtMs")?;
    Ok(())
}

fn validate_session_claim(payload: &Payload<'_>, event: &EventEnvelope) -> Result<(), EventError> {
    payload.id("claimId", validate_claim_id)?;
    payload.id("sessionId", validate_session_id)?;
    payload.operation("operationKind", &["resume", "show", "download", "poll"])?;
    let slot = payload.id("expectedSlotId", validate_slot_id)?;
    let cohort = payload.id("expectedCohort", validate_cohort)?;
    if crate::allocator::cohort_of(slot) != Some(cohort) {
        return Err(EventError::Invalid("session claim cohort"));
    }
    payload.optional_integer("expectedRuntimeOwnerGeneration", 1, 65_535)?;
    let request_id = payload.optional_id("requestId", validate_request_id)?;
    let run_id = payload.optional_id("runId", validate_run_id)?;
    if run_id.is_some() && request_id.is_none()
        || request_id != event.request_id.as_deref()
        || run_id != event.run_id.as_deref()
    {
        return Err(EventError::Invalid("session claim request binding"));
    }
    payload.literal_u64("ttlMs", 300_000)?;
    payload.cas_times("grantedAtMs")?;
    payload.id("fencingTokenSha256", validate_h256)?;
    Ok(())
}

fn validate_session_rebound(payload: &Payload<'_>) -> Result<(), EventError> {
    let session = payload.id("sessionId", validate_session_id)?;
    let expectation = payload.parse::<SessionRebindExpectation>("expectation")?;
    expectation
        .validate()
        .map_err(|_| EventError::Invalid("expectation"))?;
    let echo = payload.parse::<SessionEcho>("observedEcho")?;
    crate::session_rebind::validate_observed_echo(&expectation, &echo)
        .map_err(|_| EventError::Invalid("observedEcho"))?;
    let generation = payload.integer("pageBindingGeneration", 1, 65_535)? as u16;
    if expectation.session_id != session
        || generation
            != expectation
                .last_known_page_binding_generation
                .checked_add(1)
                .ok_or(EventError::Invalid("page binding generation"))?
        || echo.page_binding_generation != generation
    {
        return Err(EventError::Invalid("SessionRebound binding"));
    }
    payload.evidence_ref("providerReceipt")?;
    payload.timestamp("reboundAtMs")?;
    Ok(())
}

fn validate_hydration_observed(payload: &Payload<'_>) -> Result<(), EventError> {
    let session = payload.id("sessionId", validate_session_id)?;
    let observation = payload.parse::<HydrationObservation>("hydrationObservation")?;
    observation
        .validate()
        .map_err(|_| EventError::Invalid("hydrationObservation"))?;
    if observation.observed_echo.session_id != session
        || u64::from(observation.sequence_index) != payload.integer("sequenceIndex", 0, 49)?
        || observation.remaining_deadline_ms != payload.duration("remainingDeadlineMs")?
        || observation.observed_at_ms != payload.timestamp("observedAtMs")?
    {
        return Err(EventError::Invalid("hydration observation echo"));
    }
    Ok(())
}

fn validate_session_hydrated(payload: &Payload<'_>) -> Result<(), EventError> {
    let session = payload.id("sessionId", validate_session_id)?;
    let observations = payload.integer("observations", 1, 50)?;
    let terminal = payload.boolean("terminalVisible")?;
    let active = payload.boolean("activeGeneration")?;
    if payload.boolean("contentUnavailable")? || terminal == active {
        return Err(EventError::Invalid("SessionHydrated outcome"));
    }
    let final_observation = payload.parse::<HydrationObservation>("finalObservation")?;
    final_observation
        .validate()
        .map_err(|_| EventError::Invalid("finalObservation"))?;
    let state_matches = if terminal {
        matches!(
            final_observation.state,
            crate::session_rebind::hydration::HydrationState::AnswerVisible
        )
    } else {
        matches!(
            final_observation.state,
            crate::session_rebind::hydration::HydrationState::ActiveGenerationVisible
        )
    };
    if !state_matches
        || final_observation.observed_echo.session_id != session
        || u64::from(final_observation.sequence_index) + 1 != observations
    {
        return Err(EventError::Invalid("SessionHydrated final observation"));
    }
    payload.timestamp("hydratedAtMs")?;
    Ok(())
}

fn validate_answer_terminal(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("requestId", validate_request_id)?;
    payload.id("pollAttemptId", validate_operation_id)?;
    payload.id("sessionId", validate_session_id)?;
    payload.id("answerPath", validate_safe_rel_path)?;
    payload.id("answerSha256", validate_h256)?;
    payload.bytes("answerSizeBytes")?;
    payload.id("terminalAssistantTurnId", validate_turn_id)?;
    payload.evidence_ref("pollReceipt")?;
    payload.timestamp("terminalAtMs")?;
    Ok(())
}

fn validate_controls_absent(payload: &Payload<'_>) -> Result<(), EventError> {
    let claim = payload.id("artifactClaimId", validate_artifact_claim_id)?;
    let proof = payload.parse::<ZeroControlProof>("zeroControlProof")?;
    proof
        .validate_for(claim, &proof.terminal_assistant_turn_id)
        .map_err(|_| EventError::Invalid("zeroControlProof"))?;
    payload.evidence_ref("providerReceipt")?;
    payload.timestamp("observedAtMs")?;
    Ok(())
}

fn validate_controls_discovered(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("artifactClaimId", validate_artifact_claim_id)?;
    let controls = payload.array("controls", 1, 64)?;
    let count = payload.integer("controlCount", 1, 64)? as usize;
    if controls.len() != count {
        return Err(EventError::Invalid("artifact controlCount"));
    }
    let mut ids = BTreeSet::new();
    for value in controls {
        let control: ArtifactControl = serde_json::from_value(value.clone())
            .map_err(|_| EventError::Invalid("ArtifactControl"))?;
        control
            .validate_for_turn(&control.current_turn_id)
            .map_err(|_| EventError::Invalid("ArtifactControl"))?;
        if !ids.insert(control.control_id) {
            return Err(EventError::Invalid("duplicate ArtifactControl"));
        }
    }
    payload
        .parse::<BottomProof>("bottomProof")?
        .validate()
        .map_err(|_| EventError::Invalid("bottomProof"))?;
    payload.evidence_ref("providerReceipt")?;
    payload.timestamp("discoveredAtMs")?;
    Ok(())
}

fn validate_artifact_attempt(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("artifactClaimId", validate_artifact_claim_id)?;
    payload.id("attemptId", validate_operation_id)?;
    payload.integer("controlIndex", 0, 63)?;
    payload.id("controlId", validate_control_id)?;
    payload
        .parse::<ArtifactBaseline>("artifactBaseline")?
        .validate()
        .map_err(|_| EventError::Invalid("artifactBaseline"))?;
    payload.id("providerReceiptPath", validate_safe_rel_path)?;
    payload.id("hostSaveDirectory", validate_safe_rel_path)?;
    payload.literal_u64("clickBudget", 1)?;
    payload.timestamp("attemptConsumedAtMs")?;
    Ok(())
}

fn validate_artifact_download(payload: &Payload<'_>) -> Result<(), EventError> {
    let claim = payload.id("artifactClaimId", validate_artifact_claim_id)?;
    payload.id("attemptId", validate_operation_id)?;
    payload.integer("controlIndex", 0, 63)?;
    let artifact = payload.id("artifactId", validate_artifact_id)?;
    let receipt = payload.parse::<PlaywrightDownloadReceipt>("downloadReceipt")?;
    receipt
        .validate_shape()
        .map_err(|_| EventError::Invalid("downloadReceipt"))?;
    if receipt.artifact_claim_id != claim || receipt.artifact_id != artifact {
        return Err(EventError::Invalid("download receipt identity"));
    }
    payload.timestamp("completedAtMs")?;
    Ok(())
}

fn validate_artifact_completed(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("artifactClaimId", validate_artifact_claim_id)?;
    let result = payload.one_of("result", &["downloaded", "zero_controls_optional_success"])?;
    let count = payload.integer("artifactCount", 0, 64)?;
    let path = payload.optional_id("manifestPath", validate_safe_rel_path)?;
    let digest = payload.optional_id("manifestSha256", validate_h256)?;
    let valid = match result {
        "downloaded" => (1..=64).contains(&count) && path.is_some() && digest.is_some(),
        "zero_controls_optional_success" => count == 0 && path.is_none() && digest.is_none(),
        _ => false,
    };
    if !valid {
        return Err(EventError::Invalid("ArtifactClaimCompleted result"));
    }
    payload.timestamp("completedAtMs")?;
    Ok(())
}

fn validate_artifact_failed(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("artifactClaimId", validate_artifact_claim_id)?;
    payload.one_of(
        "reason",
        &[
            "artifact.required_zero",
            "artifact.controls_ambiguous",
            "artifact.bottom_unverified",
            "artifact.download_timeout",
            "artifact.event_unrecoverable",
            "artifact.integrity_failed",
            "artifact.path_unsafe",
            "contract.invalid_provider_envelope",
            "binding.mismatch",
        ],
    )?;
    payload.optional_integer("failedControlIndex", 0, 63)?;
    payload.optional_evidence("providerReceipt")?;
    payload.timestamp("failedAtMs")?;
    Ok(())
}

fn validate_release_started(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("releaseId", validate_release_id)?;
    payload.one_of("subjectKind", &["request", "session_operation", "slot"])?;
    payload.non_empty("subjectId")?;
    payload.one_of(
        "reason",
        &[
            "release.output_published",
            "release.artifact_terminal",
            "release.poll_failed",
            "release.upload_failed",
            "release.send_uncertain",
            "release.send_failed",
            "release.model_failed",
            "release.capture_failed",
            "release.session_operation_failed",
            "release.allocation_exhausted",
            "release.readiness_failed",
            "release.nonterminal_publication",
            "release.output_publish_failed",
            "release.explicit",
        ],
    )?;
    payload.timestamp("startedAtMs")?;
    Ok(())
}

fn validate_takeover(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("releaseId", validate_release_id)?;
    payload.id("slotId", validate_slot_id)?;
    let prior = payload.id("priorOwnerId", validate_owner_id)?;
    let prior_generation = payload.integer("priorGeneration", 1, 65_535)?;
    payload.id("newOwnerId", validate_owner_id)?;
    let new_generation = payload.integer("newGeneration", 1, 65_535)?;
    let proof = payload.parse::<DeadOwnerProof>("deadOwnerProof")?;
    validate_dead_owner_shape(&proof)?;
    if proof.prior_owner_id != prior
        || u64::from(proof.prior_generation) != prior_generation
        || new_generation != prior_generation + 1
        || proof.proven_at_ms != payload.timestamp("provenAtMs")?
    {
        return Err(EventError::Invalid("takeover proof binding"));
    }
    Ok(())
}

fn validate_runtime_stop_common(
    payload: &Payload<'_>,
    time: &'static str,
) -> Result<(), EventError> {
    payload.id("releaseId", validate_release_id)?;
    payload.id("runtimeOwnerId", validate_owner_id)?;
    payload.integer("ownerGeneration", 1, 65_535)?;
    payload.timestamp(time)?;
    Ok(())
}

fn validate_stop_skipped(payload: &Payload<'_>) -> Result<(), EventError> {
    payload.id("releaseId", validate_release_id)?;
    let owner = payload.optional_id("runtimeOwnerId", validate_owner_id)?;
    let reason = payload.one_of(
        "reason",
        &["runtime.owner_alive_or_unknown", "runtime.not_acquired"],
    )?;
    let proof = payload.optional_parse::<DeadOwnerProof>("proofAttempt")?;
    if let Some(proof) = &proof {
        validate_dead_owner_shape(proof)?;
    }
    match reason {
        "runtime.not_acquired" if owner.is_none() && proof.is_none() => {}
        "runtime.owner_alive_or_unknown" if owner.is_some() => {}
        _ => return Err(EventError::Invalid("RuntimeStopSkipped nullability")),
    }
    payload.timestamp("skippedAtMs")?;
    Ok(())
}

fn validate_adoption_shape(proof: &AdoptionProof) -> Result<(), EventError> {
    let pair = match (
        proof.container_label_owner_id.as_deref(),
        proof.container_label_generation,
    ) {
        (None, None) => true,
        (Some(owner), Some(generation)) => {
            validate_owner_id(owner).is_ok() && validate_generation(generation).is_ok()
        }
        _ => false,
    };
    if !pair || !docker_status(&proof.observed_docker_status) {
        return Err(EventError::Invalid("adoptionProof"));
    }
    Ok(())
}

fn validate_dead_owner_shape(proof: &DeadOwnerProof) -> Result<(), EventError> {
    let labels = match (
        proof.container_label_owner_id.as_deref(),
        proof.container_label_generation,
    ) {
        (None, None) => true,
        (Some(owner), Some(generation)) => {
            validate_owner_id(owner).is_ok() && validate_generation(generation).is_ok()
        }
        _ => false,
    };
    if validate_owner_id(&proof.prior_owner_id).is_err()
        || validate_generation(proof.prior_generation).is_err()
        || validate_timestamp_ms(proof.expired_at_ms).is_err()
        || validate_timestamp_ms(proof.grace_satisfied_at_ms).is_err()
        || validate_timestamp_ms(proof.proven_at_ms).is_err()
        || !labels
        || !(1..=8).contains(&proof.evidence_refs.len())
        || proof
            .evidence_refs
            .iter()
            .any(|value| value.validate().is_err())
    {
        return Err(EventError::Invalid("DeadOwnerProof"));
    }
    Ok(())
}

fn docker_status(value: &str) -> bool {
    matches!(
        value,
        "running" | "exited" | "missing" | "starting" | "stopping" | "unknown"
    )
}

struct Payload<'a>(&'a Map<String, Value>);

impl<'a> Payload<'a> {
    fn value(&self, key: &'static str) -> Result<&'a Value, EventError> {
        self.0.get(key).ok_or(EventError::Invalid(key))
    }

    fn text(&self, key: &'static str) -> Result<&'a str, EventError> {
        self.value(key)?.as_str().ok_or(EventError::Invalid(key))
    }

    fn non_empty(&self, key: &'static str) -> Result<&'a str, EventError> {
        let value = self.text(key)?;
        validate_non_empty_text(value).map_err(|_| EventError::Invalid(key))?;
        Ok(value)
    }

    fn id(
        &self,
        key: &'static str,
        validator: fn(&str) -> Result<(), crate::contracts::ids::IdError>,
    ) -> Result<&'a str, EventError> {
        let value = self.text(key)?;
        validator(value).map_err(|_| EventError::Invalid(key))?;
        Ok(value)
    }

    fn optional_id(
        &self,
        key: &'static str,
        validator: fn(&str) -> Result<(), crate::contracts::ids::IdError>,
    ) -> Result<Option<&'a str>, EventError> {
        match self.value(key)? {
            Value::Null => Ok(None),
            Value::String(value) => {
                validator(value).map_err(|_| EventError::Invalid(key))?;
                Ok(Some(value))
            }
            _ => Err(EventError::Invalid(key)),
        }
    }

    fn integer(&self, key: &'static str, min: u64, max: u64) -> Result<u64, EventError> {
        let value = self.value(key)?.as_u64().ok_or(EventError::Invalid(key))?;
        (min..=max)
            .contains(&value)
            .then_some(value)
            .ok_or(EventError::Invalid(key))
    }

    fn optional_integer(
        &self,
        key: &'static str,
        min: u64,
        max: u64,
    ) -> Result<Option<u64>, EventError> {
        if self.value(key)?.is_null() {
            Ok(None)
        } else {
            self.integer(key, min, max).map(Some)
        }
    }

    fn literal_u64(&self, key: &'static str, expected: u64) -> Result<(), EventError> {
        (self.integer(key, expected, expected)? == expected)
            .then_some(())
            .ok_or(EventError::Invalid(key))
    }

    fn timestamp(&self, key: &'static str) -> Result<u64, EventError> {
        let value = self.value(key)?.as_u64().ok_or(EventError::Invalid(key))?;
        validate_timestamp_ms(value).map_err(|_| EventError::Invalid(key))?;
        Ok(value)
    }

    fn optional_timestamp(&self, key: &'static str) -> Result<Option<u64>, EventError> {
        if self.value(key)?.is_null() {
            Ok(None)
        } else {
            self.timestamp(key).map(Some)
        }
    }

    fn duration(&self, key: &'static str) -> Result<u64, EventError> {
        let value = self.value(key)?.as_u64().ok_or(EventError::Invalid(key))?;
        validate_duration_ms(value).map_err(|_| EventError::Invalid(key))?;
        Ok(value)
    }

    fn bytes(&self, key: &'static str) -> Result<u64, EventError> {
        let value = self.value(key)?.as_u64().ok_or(EventError::Invalid(key))?;
        validate_byte_count(value).map_err(|_| EventError::Invalid(key))?;
        Ok(value)
    }

    fn boolean(&self, key: &'static str) -> Result<bool, EventError> {
        self.value(key)?.as_bool().ok_or(EventError::Invalid(key))
    }

    fn null(&self, key: &'static str) -> Result<(), EventError> {
        self.value(key)?
            .is_null()
            .then_some(())
            .ok_or(EventError::Invalid(key))
    }

    fn literal(&self, key: &'static str, expected: &'static str) -> Result<(), EventError> {
        (self.text(key)? == expected)
            .then_some(())
            .ok_or(EventError::Invalid(key))
    }

    fn one_of(&self, key: &'static str, values: &[&'static str]) -> Result<&'a str, EventError> {
        let value = self.text(key)?;
        values
            .contains(&value)
            .then_some(value)
            .ok_or(EventError::Invalid(key))
    }

    fn operation(&self, key: &'static str, values: &[&'static str]) -> Result<&'a str, EventError> {
        self.one_of(key, values)
    }

    fn expectation(&self, key: &'static str) -> Result<&'a str, EventError> {
        self.one_of(key, &["none", "optional", "required", "claimed"])
    }

    fn docker(&self, key: &'static str) -> Result<&'a str, EventError> {
        self.one_of(
            key,
            &[
                "running", "exited", "missing", "starting", "stopping", "unknown",
            ],
        )
    }

    fn health(&self, key: &'static str) -> Result<&'a str, EventError> {
        let value = self.text(key)?;
        crate::contracts::health::HealthStatus::parse(value)
            .map(|_| value)
            .ok_or(EventError::Invalid(key))
    }

    fn session_failure(&self, key: &'static str) -> Result<&'a str, EventError> {
        self.one_of(
            key,
            &[
                "session.rebind_failed",
                "session.pinned_slot_unavailable",
                "session.content_unavailable",
                "session.url_rejected_root",
                "session.url_rejected_mismatch",
                "session.missing",
                "session.hydration_timeout",
                "session.request_binding_missing",
                "session.claim_conflict",
                "session.provider_limit",
                "session.login_required",
                "session.subscription_required",
                "session.schema_drift",
                "contract.invalid_provider_envelope",
                "binding.mismatch",
            ],
        )
    }

    fn poll_failure(&self, key: &'static str) -> Result<&'a str, EventError> {
        self.one_of(
            key,
            &[
                "input.empty_prompt",
                "input.invalid_file",
                "input.unsupported_model_effort",
                "binding.mismatch",
                "model.control_absent",
                "model.option_absent",
                "effort.option_absent",
                "model.ambiguous",
                "provider.limit",
                "provider.login_required",
                "provider.schema_drift",
                "upload.stale_chip_mismatch",
                "upload.stale_chip_uncleared",
                "send.turn_not_proven",
                "session.rebind_failed",
                "poll.timeout",
                "provider.artifact_final_failed",
                "slot.readiness_failed",
                "runtime.owner_alive_or_unknown",
                "journal.head_cas_conflict",
                "qa.drift",
                "contract.invalid_provider_envelope",
            ],
        )
    }

    fn parse<T: for<'de> Deserialize<'de>>(&self, key: &'static str) -> Result<T, EventError> {
        serde_json::from_value(self.value(key)?.clone()).map_err(|_| EventError::Invalid(key))
    }

    fn optional_parse<T: for<'de> Deserialize<'de>>(
        &self,
        key: &'static str,
    ) -> Result<Option<T>, EventError> {
        if self.value(key)?.is_null() {
            Ok(None)
        } else {
            self.parse(key).map(Some)
        }
    }

    fn evidence_ref(&self, key: &'static str) -> Result<EvidenceRef, EventError> {
        let evidence = self.parse::<EvidenceRef>(key)?;
        evidence.validate().map_err(|_| EventError::Invalid(key))?;
        Ok(evidence)
    }

    fn optional_evidence(&self, key: &'static str) -> Result<Option<EvidenceRef>, EventError> {
        let evidence = self.optional_parse::<EvidenceRef>(key)?;
        if let Some(value) = &evidence {
            value.validate().map_err(|_| EventError::Invalid(key))?;
        }
        Ok(evidence)
    }

    fn evidence(
        &self,
        key: &'static str,
        min: usize,
        max: usize,
    ) -> Result<Vec<EvidenceRef>, EventError> {
        let values = self.parse::<Vec<EvidenceRef>>(key)?;
        if !(min..=max).contains(&values.len())
            || values.iter().any(|value| value.validate().is_err())
            || values
                .iter()
                .map(|value| &value.path)
                .collect::<BTreeSet<_>>()
                .len()
                != values.len()
        {
            return Err(EventError::Invalid(key));
        }
        Ok(values)
    }

    fn array(
        &self,
        key: &'static str,
        min: usize,
        max: usize,
    ) -> Result<&'a Vec<Value>, EventError> {
        let values = self
            .value(key)?
            .as_array()
            .ok_or(EventError::Invalid(key))?;
        (min..=max)
            .contains(&values.len())
            .then_some(values)
            .ok_or(EventError::Invalid(key))
    }

    fn id_array(
        &self,
        key: &'static str,
        validator: fn(&str) -> Result<(), crate::contracts::ids::IdError>,
        min: usize,
        max: usize,
    ) -> Result<(), EventError> {
        let values = self.array(key, min, max)?;
        let mut unique = BTreeSet::new();
        for value in values {
            let value = value.as_str().ok_or(EventError::Invalid(key))?;
            validator(value).map_err(|_| EventError::Invalid(key))?;
            if !unique.insert(value) {
                return Err(EventError::Invalid(key));
            }
        }
        Ok(())
    }

    fn cas_times(&self, base_key: &'static str) -> Result<(), EventError> {
        let base = self.timestamp(base_key)?;
        let renew = self.timestamp("renewAtMs")?;
        let expires = self.timestamp("expiresAtMs")?;
        if base.checked_add(100_000) != Some(renew) || base.checked_add(300_000) != Some(expires) {
            return Err(EventError::Invalid("CAS times"));
        }
        Ok(())
    }
}

fn closed_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, EventError> {
    let object = value
        .as_object()
        .ok_or(EventError::Invalid("nested object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    (actual == expected)
        .then_some(object)
        .ok_or(EventError::Invalid("nested object fields"))
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn validate_optional_id(
    value: Option<&str>,
    validator: fn(&str) -> Result<(), crate::contracts::ids::IdError>,
    field: &'static str,
) -> Result<(), EventError> {
    value
        .map(validator)
        .transpose()
        .map(|_| ())
        .map_err(|_| EventError::Invalid(field))
}

fn serialization(error: serde_json::Error) -> EventError {
    EventError::Serialize(error.to_string())
}
