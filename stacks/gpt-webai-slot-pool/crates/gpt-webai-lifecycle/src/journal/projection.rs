use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::allocator::cursors::advance;
use crate::contracts::events::{EventEnvelope, EventType};
use crate::contracts::ids::{h256, validate_h256};
use crate::contracts::projection::{
    AllocatorRecord, ArtifactClaimRecord, CasRecord, ProjectionContractError, ProjectionFile,
    ProjectionState, QaCounterRecord, ReleaseRecord, RequestRecord, RuntimeOwnerRecord,
    SessionRecord, SlotRecord, PROJECTION_ORDER, PROJECTION_SCHEMA,
};
use crate::journal::canonical::{canonical_bytes, parse_canonical};
use crate::journal::head::MutationGuard;
use crate::journal::replay::{topological, ReplayError};
use crate::qa::counters;
use crate::runtime::ownership::{validate_dead_owner, DeadOwnerProof};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("missing projection: {0}")]
    Missing(String),
    #[error("projection name mismatch: {0}")]
    NameMismatch(String),
    #[error("projection invalid: {0}")]
    Invalid(String),
    #[error("projection lock contended")]
    LockContended,
    #[error("unsafe projection path: {0}")]
    UnsafePath(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("projection contract error: {0}")]
    Contract(#[from] ProjectionContractError),
    #[error("journal replay error: {0}")]
    Replay(#[from] ReplayError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedSessionSeed {
    pub session_id: String,
    pub session_binding_id: Option<String>,
    pub conversation_url: String,
    pub slot_id: String,
    pub cohort: String,
    pub page_binding_generation: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReducedProjection {
    pub state: ProjectionState,
    pub files: BTreeMap<String, ProjectionFile>,
}

#[derive(Clone, Debug)]
pub struct ProjectionStore {
    state_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaimRole {
    Request,
    Session,
}

#[derive(Default)]
struct ActiveReleaseSources {
    request_claim: Option<String>,
    session_claim: Option<String>,
    slot_lease: Option<String>,
    runtime_owner: Option<String>,
}

impl ActiveReleaseSources {
    fn event_ids(&self) -> Vec<String> {
        [
            self.request_claim.as_ref(),
            self.session_claim.as_ref(),
            self.slot_lease.as_ref(),
            self.runtime_owner.as_ref(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }
}

struct Reducer<'a> {
    state: ProjectionState,
    file_last: BTreeMap<&'static str, Option<String>>,
    seeds: &'a BTreeMap<String, PersistedSessionSeed>,
    claim_roles: BTreeMap<String, ClaimRole>,
    lease_claims: BTreeMap<String, String>,
    owner_leases: BTreeMap<String, String>,
}

pub fn empty_files() -> BTreeMap<String, ProjectionFile> {
    PROJECTION_ORDER
        .into_iter()
        .map(|name| {
            (
                name.to_string(),
                ProjectionFile {
                    projection_name: name.to_string(),
                    last_event_id: None,
                    records: BTreeMap::new(),
                },
            )
        })
        .collect()
}

pub fn empty_state() -> Result<ProjectionState, ProjectionError> {
    let files = empty_files();
    Ok(ProjectionState {
        allocator: BTreeMap::new(),
        artifact_claims: BTreeMap::new(),
        claims: BTreeMap::new(),
        last_event_created_at_ms: 0,
        last_event_id: None,
        leases: BTreeMap::new(),
        projection_digest: projection_digest(&files)?,
        qa_counters: BTreeMap::new(),
        releases: BTreeMap::new(),
        requests: BTreeMap::new(),
        runtime_owners: BTreeMap::new(),
        schema_version: PROJECTION_SCHEMA.to_string(),
        sessions: BTreeMap::new(),
        slots: BTreeMap::new(),
    })
}

pub fn reduce(
    events: &[EventEnvelope],
    seeds: &BTreeMap<String, PersistedSessionSeed>,
) -> Result<ReducedProjection, ProjectionError> {
    let ordered = topological(events)?;
    let mut reducer = Reducer::new(seeds)?;
    for event in &ordered {
        reducer.apply(event)?;
    }
    reducer.finish()
}

pub fn projection_digest(
    files: &BTreeMap<String, ProjectionFile>,
) -> Result<String, ProjectionError> {
    let mut bytes = Vec::new();
    for name in PROJECTION_ORDER {
        let file = files
            .get(name)
            .ok_or_else(|| ProjectionError::Missing(name.to_string()))?;
        if file.projection_name != name {
            return Err(ProjectionError::NameMismatch(name.to_string()));
        }
        file.validate()?;
        bytes.extend(canonical_bytes(file)?);
    }
    if files.len() != PROJECTION_ORDER.len() {
        return Err(ProjectionError::Invalid(
            "unexpected projection file".to_string(),
        ));
    }
    Ok(h256(bytes))
}

impl ProjectionStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn directory(&self) -> PathBuf {
        self.state_root.join("journal/projections")
    }

    pub fn publish(
        &self,
        _guard: &MutationGuard,
        operation_id: &str,
        projection: &ReducedProjection,
    ) -> Result<(), ProjectionError> {
        if projection.state.projection_digest != projection_digest(&projection.files)? {
            return Err(ProjectionError::Invalid("projectionDigest".to_string()));
        }
        let directory = self.directory();
        crate::provider_runner::create_private_directory(&self.state_root, &directory)?;
        let locks = self.state_root.join("journal/locks");
        crate::provider_runner::create_private_directory(&self.state_root, &locks)?;
        let lock = ProjectionLock::acquire(self.state_root.join("journal/locks/projection.lock"))?;
        let mut staged = Vec::with_capacity(PROJECTION_ORDER.len());
        for name in PROJECTION_ORDER {
            let file = projection
                .files
                .get(name)
                .ok_or_else(|| ProjectionError::Missing(name.to_string()))?;
            let bytes = canonical_bytes(file)?;
            let target = directory.join(format!("{name}.json"));
            let temp = directory.join(format!(
                ".{name}.{operation_id}.{}.{}.tmp",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            write_private(&temp, &bytes)?;
            staged.push((temp, target, bytes));
        }
        for (temp, target, _) in &staged {
            fs::rename(temp, target)?;
        }
        File::open(&directory)?.sync_all()?;
        for (_, target, expected) in staged {
            let actual = read_private(&target)?;
            if actual != expected {
                return Err(ProjectionError::Invalid(format!(
                    "projection reopen mismatch: {}",
                    target.display()
                )));
            }
        }
        drop(lock);
        let reopened = self.read_all()?;
        if reopened != projection.files {
            return Err(ProjectionError::Invalid(
                "published projection mismatch".to_string(),
            ));
        }
        Ok(())
    }

    pub fn read_all(&self) -> Result<BTreeMap<String, ProjectionFile>, ProjectionError> {
        let directory = self.directory();
        let mut files = BTreeMap::new();
        for name in PROJECTION_ORDER {
            let path = directory.join(format!("{name}.json"));
            let bytes = read_private(&path)?;
            parse_canonical(&bytes)?;
            let file: ProjectionFile = serde_json::from_slice(&bytes)?;
            file.validate()?;
            if file.projection_name != name {
                return Err(ProjectionError::NameMismatch(name.to_string()));
            }
            files.insert(name.to_string(), file);
        }
        projection_digest(&files)?;
        Ok(files)
    }
}

impl<'a> Reducer<'a> {
    fn new(seeds: &'a BTreeMap<String, PersistedSessionSeed>) -> Result<Self, ProjectionError> {
        Ok(Self {
            state: empty_state()?,
            file_last: PROJECTION_ORDER
                .into_iter()
                .map(|name| (name, None))
                .collect(),
            seeds,
            claim_roles: BTreeMap::new(),
            lease_claims: BTreeMap::new(),
            owner_leases: BTreeMap::new(),
        })
    }

    fn apply(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        self.bootstrap_session(event)?;
        use EventType::*;
        match event.event_type {
            RequestAccepted => self.request_accepted(event)?,
            RequestClaimGranted => self.claim_granted(event, ClaimRole::Request)?,
            RequestClaimRenewed | SessionOperationClaimRenewed => self.claim_renewed(event)?,
            HostAttachmentsStaged => self.request_state(event, None)?,
            AllocationCandidateObserved => self.allocation_observed(event)?,
            AllocationExhausted => {
                self.allocator_last(event, Some(9))?;
                self.request_state(event, Some("failed"))?;
            }
            SlotLeaseGranted => self.lease_granted(event)?,
            SlotLeaseRenewed => self.lease_renewed(event)?,
            RuntimeOwnershipGranted
            | RuntimeOwnershipAdopted
            | SessionRuntimeOwnershipGranted
            | SessionRuntimeOwnershipAdopted => self.owner_granted(event)?,
            RuntimeOwnershipRenewed => self.owner_renewed(event)?,
            SlotHealthProbeStarted => self.slot_probe_started(event)?,
            SlotHealthObserved => self.slot_health_observed(event)?,
            RootCaptureStarted => self.request_state(event, Some("binding"))?,
            RootCaptureObserved
            | ModelSelectionStarted
            | UploadStarted
            | UploadMismatchObserved
            | UploadCleared
            | UploadCompleted
            | PollProgress
            | TerminalPersisted => self.request_state(event, None)?,
            RootCaptureFailed | ModelSelectionFailed | UploadFailed | SendUncertain
            | SendFailed | PollFailed | OutputPublishFailed => {
                self.request_state(event, Some("failed"))?
            }
            ModelSelectionVerified => self.request_state(event, Some("model_verified"))?,
            SlotAttachmentsMaterialized => self.request_state(event, Some("uploading"))?,
            SendClickArmed => self.request_state(event, Some("send_armed"))?,
            SendClicked | SendReconciled => self.request_state(event, Some("sent"))?,
            TurnStartConfirmed => self.turn_confirmed(event)?,
            SessionBindingEstablished => self.session_binding(event)?,
            RunningProjected => self.request_state(event, Some("running"))?,
            SessionOperationClaimGranted => self.claim_granted(event, ClaimRole::Session)?,
            PersistedSessionLeaseGranted => self.lease_granted(event)?,
            SessionRebindStarted | SessionOperationFailed => self.session_operation(event)?,
            SessionRebound => self.session_rebound(event)?,
            SessionHydrationObserved | SessionHydrated => self.session_touch(event)?,
            PollStarted => self.request_state(event, Some("polling"))?,
            AnswerTerminal => self.answer_terminal(event)?,
            ArtifactClaimEstablished => self.artifact_claim_started(event)?,
            ArtifactControlsAbsent => self.artifact_control_count(event, 0)?,
            ArtifactControlsDiscovered => {
                self.artifact_control_count(event, u8_field(event, "controlCount")?)?
            }
            ArtifactDownloadAttemptConsumed => self.artifact_attempt(event)?,
            ArtifactRecoveryCandidateObserved | ArtifactDownloadCompleted => {
                self.artifact_touch(event)?
            }
            ArtifactClaimCompleted => self.artifact_completed(event, true)?,
            ArtifactClaimFailed => self.artifact_completed(event, false)?,
            OutputPublished => self.request_state(event, Some("output_published"))?,
            ReleaseStarted => self.release_started(event)?,
            ReleaseEvidencePreserved => self.release_evidence(event)?,
            RuntimeTakeoverProven => self.owner_takeover(event)?,
            RuntimeStopStarted => self.release_runtime(event, "pending")?,
            RuntimeStopped => self.release_runtime(event, "stopped")?,
            RuntimeStopFailed => self.release_runtime(event, "failed")?,
            RuntimeStopSkipped => self.release_runtime(event, "skipped")?,
            ReleaseCleanupStarted => self.release_touch(event)?,
            ReleaseCleanupFailed => self.release_cleanup_failed(event)?,
            SessionOperationClaimReleased => self.claim_released(event, "sessionClaimRelease")?,
            RequestClaimReleased => self.claim_released(event, "requestClaimRelease")?,
            SlotLeaseReleased => self.lease_released(event)?,
            RuntimeOwnershipReleased => self.owner_released(event)?,
            ReleaseCleanupCommitted => self.release_cleanup_committed(event)?,
            SlotStandbyWritten => self.slot_standby(event)?,
            ReleaseCooldownBlocked => self.release_cooldown(event)?,
            CooldownCleared => self.cooldown_cleared(event)?,
            ReleaseFinalized => self.release_finalized(event)?,
            SnapshotPublished | SnapshotRejected => {}
            QaMatrixRecorded => self.qa_matrix(event)?,
            QaRepeatRecorded => self.qa_repeat(event)?,
            QaCountersReset => self.qa_reset(event)?,
        }
        self.state.last_event_id = Some(event.event_id.clone());
        self.state.last_event_created_at_ms = event.created_at_ms;
        Ok(())
    }

    fn finish(mut self) -> Result<ReducedProjection, ProjectionError> {
        let files = self.files()?;
        self.state.projection_digest = projection_digest(&files)?;
        Ok(ReducedProjection {
            state: self.state,
            files,
        })
    }

    fn files(&self) -> Result<BTreeMap<String, ProjectionFile>, ProjectionError> {
        let mut files = BTreeMap::new();
        files.insert(
            "requests".to_string(),
            self.file("requests", &self.state.requests)?,
        );
        files.insert(
            "sessions".to_string(),
            self.file("sessions", &self.state.sessions)?,
        );
        files.insert("slots".to_string(), self.file("slots", &self.state.slots)?);
        files.insert(
            "allocator".to_string(),
            self.file("allocator", &self.state.allocator)?,
        );
        files.insert(
            "claims".to_string(),
            self.file("claims", &self.state.claims)?,
        );
        files.insert(
            "leases".to_string(),
            self.file("leases", &self.state.leases)?,
        );
        files.insert(
            "runtime_owners".to_string(),
            self.file("runtime_owners", &self.state.runtime_owners)?,
        );
        files.insert(
            "artifact_claims".to_string(),
            self.file("artifact_claims", &self.state.artifact_claims)?,
        );
        files.insert(
            "releases".to_string(),
            self.file("releases", &self.state.releases)?,
        );
        files.insert(
            "qa_counters".to_string(),
            self.file("qa_counters", &self.state.qa_counters)?,
        );
        Ok(files)
    }

    fn file<T: Serialize>(
        &self,
        name: &'static str,
        records: &BTreeMap<String, T>,
    ) -> Result<ProjectionFile, ProjectionError> {
        let records = records
            .iter()
            .map(|(key, value)| Ok((key.clone(), serde_json::to_value(value)?)))
            .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
        let file = ProjectionFile {
            projection_name: name.to_string(),
            last_event_id: self.file_last[name].clone(),
            records,
        };
        file.validate()?;
        Ok(file)
    }

    fn touch(&mut self, name: &'static str, event: &EventEnvelope) {
        self.file_last.insert(name, Some(event.event_id.clone()));
    }

    fn request_accepted(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let request_id = string_field(event, "requestId")?;
        if self.state.requests.contains_key(&request_id) {
            return invalid("duplicate request record");
        }
        self.state.requests.insert(
            request_id.clone(),
            RequestRecord {
                request_id,
                kind: string_field(event, "kind")?,
                state: "accepted".to_string(),
                session_id: None,
                last_event_id: event.event_id.clone(),
            },
        );
        self.touch("requests", event);
        Ok(())
    }

    fn request_state(
        &mut self,
        event: &EventEnvelope,
        state: Option<&str>,
    ) -> Result<(), ProjectionError> {
        let request_id = event
            .request_id
            .as_deref()
            .ok_or_else(|| ProjectionError::Invalid("requestId missing".to_string()))?;
        let record =
            self.state.requests.get_mut(request_id).ok_or_else(|| {
                ProjectionError::Invalid(format!("unknown request: {request_id}"))
            })?;
        if let Some(state) = state {
            record.state = state.to_string();
        }
        record.last_event_id = event.event_id.clone();
        self.touch("requests", event);
        Ok(())
    }

    fn turn_confirmed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        self.request_state(event, Some("sent"))?;
        let request_id = event
            .request_id
            .as_deref()
            .expect("validated request event");
        self.state
            .requests
            .get_mut(request_id)
            .expect("known request")
            .session_id = Some(string_field(event, "sessionId")?);
        Ok(())
    }

    fn claim_granted(
        &mut self,
        event: &EventEnvelope,
        role: ClaimRole,
    ) -> Result<(), ProjectionError> {
        let claim_id = string_field(event, "claimId")?;
        let subject_id = match role {
            ClaimRole::Request => string_field(event, "requestId")?,
            ClaimRole::Session => string_field(event, "sessionId")?,
        };
        ensure_no_active(&self.state.claims, &subject_id)?;
        let generation = if role == ClaimRole::Request {
            u16_field(event, "claimGeneration")?
        } else {
            1
        };
        let record = cas_grant(
            event,
            claim_id.clone(),
            "claim",
            subject_id,
            generation,
            "grantedAtMs",
        )?;
        if self.state.claims.insert(claim_id.clone(), record).is_some() {
            return invalid("duplicate claim id");
        }
        self.claim_roles.insert(claim_id, role);
        self.touch("claims", event);
        if role == ClaimRole::Request {
            self.request_state(event, Some("claimed"))?;
        }
        Ok(())
    }

    fn claim_renewed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "claimId")?;
        renew_cas(
            self.state
                .claims
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("unknown claim".to_string()))?,
            event,
            "claimGeneration",
            "renewedAtMs",
        )?;
        self.touch("claims", event);
        Ok(())
    }

    fn allocation_observed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let mut record = self
            .state
            .allocator
            .remove("allocator")
            .unwrap_or_else(|| AllocatorRecord::zeroed(event.event_id.clone()));
        let ordinal = u8_field(event, "scanOrdinal")?;
        let candidate = advance(&mut record, ordinal);
        if candidate.cohort != string_field(event, "cohort")?
            || candidate.slot_id != string_field(event, "slotId")?
            || candidate.cohort_cursor_before != u8_field(event, "cohortCursorBefore")?
            || candidate.within_cursor_before != u8_field(event, "withinCursorBefore")?
        {
            return invalid("allocator observation does not match cursors");
        }
        record.last_event_id = event.event_id.clone();
        self.state.allocator.insert("allocator".to_string(), record);
        self.touch("allocator", event);
        Ok(())
    }

    fn allocator_last(
        &mut self,
        event: &EventEnvelope,
        ordinal: Option<u8>,
    ) -> Result<(), ProjectionError> {
        let record = self
            .state
            .allocator
            .get_mut("allocator")
            .ok_or_else(|| ProjectionError::Invalid("allocator missing".to_string()))?;
        if let Some(ordinal) = ordinal {
            record.last_scan_ordinal = Some(ordinal);
        }
        record.last_event_id = event.event_id.clone();
        self.touch("allocator", event);
        Ok(())
    }

    fn lease_granted(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let lease_id = string_field(event, "leaseId")?;
        let claim_id = string_field(event, "claimId")?;
        if !self.state.claims.contains_key(&claim_id) {
            return invalid("lease claim missing");
        }
        let slot_id = string_field(event, "slotId")?;
        ensure_no_active(&self.state.leases, &slot_id)?;
        let record = cas_grant(
            event,
            lease_id.clone(),
            "lease",
            slot_id,
            u16_field(event, "leaseGeneration")?,
            "grantedAtMs",
        )?;
        if self.state.leases.insert(lease_id.clone(), record).is_some() {
            return invalid("duplicate lease id");
        }
        self.lease_claims.insert(lease_id, claim_id);
        self.touch("leases", event);
        if event.event_type == EventType::SlotLeaseGranted {
            self.request_state(event, Some("allocated"))?;
        }
        Ok(())
    }

    fn lease_renewed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "leaseId")?;
        renew_cas(
            self.state
                .leases
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("unknown lease".to_string()))?,
            event,
            "leaseGeneration",
            "renewedAtMs",
        )?;
        self.touch("leases", event);
        Ok(())
    }

    fn owner_granted(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "runtimeOwnerId")?;
        let lease_id = string_field(event, "leaseId")?;
        if !self.state.leases.contains_key(&lease_id) {
            return invalid("owner lease missing");
        }
        let slot_id = string_field(event, "slotId")?;
        ensure_no_active_owners(&self.state.runtime_owners, &slot_id)?;
        let time_field = if matches!(
            event.event_type,
            EventType::RuntimeOwnershipAdopted | EventType::SessionRuntimeOwnershipAdopted
        ) {
            "adoptedAtMs"
        } else {
            "grantedAtMs"
        };
        let cas = cas_grant(
            event,
            id.clone(),
            "runtime_owner",
            slot_id,
            u16_field(event, "ownerGeneration")?,
            time_field,
        )?;
        let record = RuntimeOwnerRecord {
            cas,
            runtime_incarnation_id: string_field(event, "runtimeIncarnationId")?,
            docker_status: string_field(event, "dockerStatus")?,
        };
        if self
            .state
            .runtime_owners
            .insert(id.clone(), record)
            .is_some()
        {
            return invalid("duplicate runtime owner id");
        }
        self.owner_leases.insert(id, lease_id);
        self.touch("runtime_owners", event);
        Ok(())
    }

    fn owner_renewed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "runtimeOwnerId")?;
        renew_cas(
            &mut self
                .state
                .runtime_owners
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("unknown owner".to_string()))?
                .cas,
            event,
            "ownerGeneration",
            "renewedAtMs",
        )?;
        self.touch("runtime_owners", event);
        Ok(())
    }

    fn slot_probe_started(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let slot_id = string_field(event, "slotId")?;
        let cohort = crate::allocator::cohort_of(&slot_id)
            .ok_or_else(|| ProjectionError::Invalid("unknown physical slot".to_string()))?;
        let record = self
            .state
            .slots
            .entry(slot_id.clone())
            .or_insert(SlotRecord {
                slot_id,
                cohort: cohort.to_string(),
                health_status: "unknown".to_string(),
                docker_status: "unknown".to_string(),
                allocatable: false,
                cooldown_until_ms: None,
                standby: false,
                last_event_id: event.event_id.clone(),
            });
        record.last_event_id = event.event_id.clone();
        self.touch("slots", event);
        Ok(())
    }

    fn slot_health_observed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let slot_id = string_field(event, "slotId")?;
        let cooldown = u64_field(event, "cooldownMs")?;
        let observed = u64_field(event, "observedAtMs")?;
        let record = self
            .state
            .slots
            .get_mut(&slot_id)
            .ok_or_else(|| ProjectionError::Invalid("slot probe missing".to_string()))?;
        record.health_status = string_field(event, "healthStatus")?;
        record.docker_status = string_field(event, "dockerStatus")?;
        record.allocatable = bool_field(event, "allocatable")?;
        record.cooldown_until_ms = (cooldown != 0)
            .then(|| observed.checked_add(cooldown))
            .flatten()
            .ok_or_else(|| ProjectionError::Invalid("cooldown overflow".to_string()))
            .map(Some)
            .or_else(|error| if cooldown == 0 { Ok(None) } else { Err(error) })?;
        record.last_event_id = event.event_id.clone();
        self.touch("slots", event);
        Ok(())
    }

    fn session_binding(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "sessionId")?;
        if self.state.sessions.contains_key(&id) {
            return invalid("duplicate session binding");
        }
        self.state.sessions.insert(
            id.clone(),
            SessionRecord {
                session_id: id,
                session_binding_id: Some(string_field(event, "sessionBindingId")?),
                conversation_url: Some(string_field(event, "conversationUrl")?),
                slot_id: string_field(event, "slotId")?,
                cohort: string_field(event, "cohort")?,
                page_binding_generation: u16_field(event, "pageBindingGeneration")?,
                last_operation_kind: Some("run".to_string()),
                terminal_answer_sha256: None,
                last_event_id: event.event_id.clone(),
            },
        );
        self.touch("sessions", event);
        Ok(())
    }

    fn bootstrap_session(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        use EventType::*;
        if event.aggregate.kind != crate::contracts::events::AggregateKind::Session
            || event.event_type == SessionBindingEstablished
            || self.state.sessions.contains_key(&event.aggregate.id)
        {
            return Ok(());
        }
        let seed = self.seeds.get(&event.aggregate.id).ok_or_else(|| {
            ProjectionError::Invalid(format!(
                "persisted session seed missing: {}",
                event.aggregate.id
            ))
        })?;
        if seed.session_id != event.aggregate.id {
            return invalid("persisted session seed identity");
        }
        let operation_kind = match event.event_type {
            SessionRebindStarted | SessionOperationFailed => string_field(event, "operationKind")?,
            _ => return invalid("illegal persisted-session bootstrap event"),
        };
        self.state.sessions.insert(
            seed.session_id.clone(),
            SessionRecord {
                session_id: seed.session_id.clone(),
                session_binding_id: seed.session_binding_id.clone(),
                conversation_url: Some(seed.conversation_url.clone()),
                slot_id: seed.slot_id.clone(),
                cohort: seed.cohort.clone(),
                page_binding_generation: seed.page_binding_generation.unwrap_or(1),
                last_operation_kind: Some(operation_kind),
                terminal_answer_sha256: None,
                last_event_id: event.event_id.clone(),
            },
        );
        self.touch("sessions", event);
        Ok(())
    }

    fn session_operation(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let session_id = string_field(event, "sessionId")?;
        let record = self
            .state
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| ProjectionError::Invalid("session missing".to_string()))?;
        record.last_operation_kind = Some(string_field(event, "operationKind")?);
        record.last_event_id = event.event_id.clone();
        self.touch("sessions", event);
        Ok(())
    }

    fn session_rebound(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let session_id = string_field(event, "sessionId")?;
        let record = self
            .state
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| ProjectionError::Invalid("session missing".to_string()))?;
        let generation = u16_field(event, "pageBindingGeneration")?;
        if generation != record.page_binding_generation.saturating_add(1) {
            return invalid("page binding generation");
        }
        record.page_binding_generation = generation;
        record.last_event_id = event.event_id.clone();
        self.touch("sessions", event);
        Ok(())
    }

    fn session_touch(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "sessionId")?;
        self.state
            .sessions
            .get_mut(&id)
            .ok_or_else(|| ProjectionError::Invalid("session missing".to_string()))?
            .last_event_id = event.event_id.clone();
        self.touch("sessions", event);
        Ok(())
    }

    fn answer_terminal(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        self.request_state(event, Some("terminal"))?;
        let session_id = string_field(event, "sessionId")?;
        let session = self
            .state
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| ProjectionError::Invalid("session missing".to_string()))?;
        session.terminal_answer_sha256 = Some(string_field(event, "answerSha256")?);
        session.last_event_id = event.event_id.clone();
        self.touch("sessions", event);
        Ok(())
    }

    fn artifact_claim_started(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "artifactClaimId")?;
        let request_id = optional_string_field(event, "requestId")?;
        let record = ArtifactClaimRecord {
            artifact_claim_id: id.clone(),
            session_id: string_field(event, "sessionId")?,
            request_id,
            expectation: string_field(event, "expectation")?,
            control_count: None,
            attempts_consumed: 0,
            completed: false,
            result: None,
            last_event_id: event.event_id.clone(),
        };
        if self.state.artifact_claims.insert(id, record).is_some() {
            return invalid("duplicate artifact claim");
        }
        self.touch("artifact_claims", event);
        Ok(())
    }

    fn artifact_control_count(
        &mut self,
        event: &EventEnvelope,
        count: u8,
    ) -> Result<(), ProjectionError> {
        let record = self.artifact_mut(event)?;
        record.control_count = Some(count);
        record.last_event_id = event.event_id.clone();
        self.touch("artifact_claims", event);
        Ok(())
    }

    fn artifact_attempt(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let record = self.artifact_mut(event)?;
        record.attempts_consumed = record
            .attempts_consumed
            .checked_add(1)
            .filter(|count| *count <= 64)
            .ok_or_else(|| ProjectionError::Invalid("artifact attempt overflow".to_string()))?;
        record.last_event_id = event.event_id.clone();
        self.touch("artifact_claims", event);
        Ok(())
    }

    fn artifact_touch(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        self.artifact_mut(event)?.last_event_id = event.event_id.clone();
        self.touch("artifact_claims", event);
        Ok(())
    }

    fn artifact_completed(
        &mut self,
        event: &EventEnvelope,
        success: bool,
    ) -> Result<(), ProjectionError> {
        let result = success.then(|| string_field(event, "result")).transpose()?;
        let record = self.artifact_mut(event)?;
        record.completed = true;
        if success {
            record.result = result;
        }
        record.last_event_id = event.event_id.clone();
        self.touch("artifact_claims", event);
        Ok(())
    }

    fn artifact_mut(
        &mut self,
        event: &EventEnvelope,
    ) -> Result<&mut ArtifactClaimRecord, ProjectionError> {
        let id = string_field(event, "artifactClaimId")?;
        self.state
            .artifact_claims
            .get_mut(&id)
            .ok_or_else(|| ProjectionError::Invalid("artifact claim missing".to_string()))
    }

    fn release_started(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "releaseId")?;
        let subject_kind = string_field(event, "subjectKind")?;
        let subject_id = string_field(event, "subjectId")?;
        let applicable = self.active_release_sources(&subject_kind, &subject_id)?;
        if string_field(event, "reason")? == "release.explicit"
            && event.source_event_ids != applicable.event_ids()
        {
            return invalid("explicit release sources are not current active resources");
        }
        let record = ReleaseRecord {
            release_id: id.clone(),
            subject_kind,
            subject_id,
            reason: string_field(event, "reason")?,
            started_at_ms: u64_field(event, "startedAtMs")?,
            evidence_preserved_event_id: None,
            runtime_outcome: "pending".to_string(),
            request_claim_release: mode(applicable.request_claim.is_some()),
            session_claim_release: mode(applicable.session_claim.is_some()),
            slot_lease_release: mode(applicable.slot_lease.is_some()),
            runtime_owner_release: mode(applicable.runtime_owner.is_some()),
            standby_written: false,
            final_status: None,
            finalized_at_ms: None,
            last_event_id: event.event_id.clone(),
        };
        if self.state.releases.insert(id, record).is_some() {
            return invalid("duplicate release");
        }
        self.touch("releases", event);
        Ok(())
    }

    fn active_release_sources(
        &self,
        subject_kind: &str,
        subject_id: &str,
    ) -> Result<ActiveReleaseSources, ProjectionError> {
        let mut related_claims = BTreeSet::new();
        for (id, record) in &self.state.claims {
            let role = self.claim_roles.get(id);
            if record.subject_id == subject_id
                && matches!(
                    (subject_kind, role),
                    ("request", Some(ClaimRole::Request))
                        | ("session_operation", Some(ClaimRole::Session))
                )
            {
                related_claims.insert(id.clone());
            }
        }
        let mut related_leases = BTreeSet::new();
        for (id, record) in &self.state.leases {
            let direct_slot = subject_kind == "slot" && record.subject_id == subject_id;
            let linked = self
                .lease_claims
                .get(id)
                .is_some_and(|claim| related_claims.contains(claim));
            if direct_slot || linked {
                related_leases.insert(id.clone());
                if let Some(claim) = self.lease_claims.get(id) {
                    related_claims.insert(claim.clone());
                }
            }
        }
        let mut sources = ActiveReleaseSources::default();
        for claim_id in related_claims {
            let Some(record) = self.state.claims.get(&claim_id) else {
                continue;
            };
            if record.status != "active" {
                continue;
            }
            match self.claim_roles.get(&claim_id) {
                Some(ClaimRole::Request) => set_unique_source(
                    &mut sources.request_claim,
                    &record.last_event_id,
                    "multiple active request claims",
                )?,
                Some(ClaimRole::Session) => set_unique_source(
                    &mut sources.session_claim,
                    &record.last_event_id,
                    "multiple active session claims",
                )?,
                None => return invalid("claim role missing"),
            }
        }
        for lease_id in &related_leases {
            let Some(record) = self.state.leases.get(lease_id) else {
                continue;
            };
            if record.status == "active" {
                set_unique_source(
                    &mut sources.slot_lease,
                    &record.last_event_id,
                    "multiple active slot leases",
                )?;
            }
        }
        for (owner_id, record) in &self.state.runtime_owners {
            let linked = self
                .owner_leases
                .get(owner_id)
                .is_some_and(|lease| related_leases.contains(lease));
            let direct_slot = subject_kind == "slot" && record.cas.subject_id == subject_id;
            if record.cas.status == "active" && (linked || direct_slot) {
                set_unique_source(
                    &mut sources.runtime_owner,
                    &record.cas.last_event_id,
                    "multiple active runtime owners",
                )?;
            }
        }
        Ok(sources)
    }

    fn release_evidence(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let release = self.release_mut(event)?;
        release.evidence_preserved_event_id = Some(event.event_id.clone());
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn release_runtime(
        &mut self,
        event: &EventEnvelope,
        outcome: &str,
    ) -> Result<(), ProjectionError> {
        let release = self.release_mut(event)?;
        release.runtime_outcome = outcome.to_string();
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn release_touch(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        self.release_mut(event)?.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn release_cleanup_failed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let release = self.release_mut(event)?;
        release.final_status = Some("cleanup_failed".to_string());
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn release_cleanup_committed(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let release = self.release_mut(event)?;
        release.request_claim_release = string_field(event, "requestClaimReleaseMode")?;
        release.session_claim_release = string_field(event, "sessionClaimReleaseMode")?;
        release.slot_lease_release = string_field(event, "leaseReleaseMode")?;
        release.runtime_owner_release = string_field(event, "ownerReleaseMode")?;
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn claim_released(
        &mut self,
        event: &EventEnvelope,
        release_field: &str,
    ) -> Result<(), ProjectionError> {
        let id = string_field(event, "claimId")?;
        release_cas(
            self.state
                .claims
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("claim missing".to_string()))?,
            event,
            "claimGeneration",
            "releasedAtMs",
        )?;
        self.touch("claims", event);
        let release = self.release_mut(event)?;
        match release_field {
            "requestClaimRelease" => release.request_claim_release = "released".to_string(),
            "sessionClaimRelease" => release.session_claim_release = "released".to_string(),
            _ => return invalid("release field"),
        }
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn lease_released(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "leaseId")?;
        release_cas(
            self.state
                .leases
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("lease missing".to_string()))?,
            event,
            "leaseGeneration",
            "releasedAtMs",
        )?;
        self.touch("leases", event);
        let release = self.release_mut(event)?;
        release.slot_lease_release = "released".to_string();
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn owner_released(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "runtimeOwnerId")?;
        release_cas(
            &mut self
                .state
                .runtime_owners
                .get_mut(&id)
                .ok_or_else(|| ProjectionError::Invalid("owner missing".to_string()))?
                .cas,
            event,
            "ownerGeneration",
            "releasedAtMs",
        )?;
        self.touch("runtime_owners", event);
        let release = self.release_mut(event)?;
        release.runtime_owner_release = "released".to_string();
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn owner_takeover(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let prior_id = string_field(event, "priorOwnerId")?;
        let new_id = string_field(event, "newOwnerId")?;
        let proof: DeadOwnerProof = serde_json::from_value(
            object(event)?
                .get("deadOwnerProof")
                .cloned()
                .ok_or_else(|| ProjectionError::Invalid("deadOwnerProof".to_string()))?,
        )?;
        let prior = self
            .state
            .runtime_owners
            .get(&prior_id)
            .cloned()
            .ok_or_else(|| ProjectionError::Invalid("prior owner missing".to_string()))?;
        let slot_id = string_field(event, "slotId")?;
        if prior.cas.subject_id != slot_id {
            return invalid("takeover slot binding");
        }
        validate_dead_owner(&prior, &proof)
            .map_err(|error| ProjectionError::Invalid(error.to_string()))?;
        let prior_generation = u16_field(event, "priorGeneration")?;
        let new_generation = u16_field(event, "newGeneration")?;
        if prior.cas.generation != prior_generation
            || new_generation
                != prior_generation.checked_add(1).ok_or_else(|| {
                    ProjectionError::Invalid("owner generation overflow".to_string())
                })?
            || proof.proven_at_ms != u64_field(event, "provenAtMs")?
        {
            return invalid("takeover generation/time");
        }
        let mut retired = prior.clone();
        retired.cas.status = "released".to_string();
        retired.cas.released_at_ms = Some(proof.proven_at_ms);
        retired.cas.release_event_id = Some(event.event_id.clone());
        retired.cas.last_event_id = event.event_id.clone();
        let replacement = RuntimeOwnerRecord {
            cas: CasRecord {
                id: new_id.clone(),
                kind: "runtime_owner".to_string(),
                subject_id: slot_id,
                owner: event.writer.clone(),
                generation: new_generation,
                renewal_revision: 1,
                fencing_token_sha256: None,
                granted_at_ms: proof.proven_at_ms,
                renew_at_ms: proof.proven_at_ms.checked_add(100_000).ok_or_else(|| {
                    ProjectionError::Invalid("takeover time overflow".to_string())
                })?,
                expires_at_ms: proof.proven_at_ms.checked_add(300_000).ok_or_else(|| {
                    ProjectionError::Invalid("takeover time overflow".to_string())
                })?,
                status: "active".to_string(),
                released_at_ms: None,
                release_event_id: None,
                last_event_id: event.event_id.clone(),
            },
            runtime_incarnation_id: prior.runtime_incarnation_id,
            docker_status: prior.docker_status,
        };
        self.state.runtime_owners.insert(prior_id.clone(), retired);
        if self
            .state
            .runtime_owners
            .insert(new_id.clone(), replacement)
            .is_some()
        {
            return invalid("new owner already exists");
        }
        if let Some(lease) = self.owner_leases.get(&prior_id).cloned() {
            self.owner_leases.insert(new_id, lease);
        }
        self.touch("runtime_owners", event);
        Ok(())
    }

    fn slot_standby(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let slot_id = string_field(event, "slotId")?;
        let release_id = string_field(event, "releaseId")?;
        let runtime_failed = self
            .state
            .releases
            .get(&release_id)
            .ok_or_else(|| ProjectionError::Invalid("release missing".to_string()))?
            .runtime_outcome
            == "failed";
        if runtime_failed && bool_field(event, "allocatable")? {
            return invalid("runtime stop failure cannot write allocatable standby");
        }
        let record = self
            .state
            .slots
            .get_mut(&slot_id)
            .ok_or_else(|| ProjectionError::Invalid("slot missing".to_string()))?;
        record.standby = true;
        record.allocatable = bool_field(event, "allocatable")?;
        record.cooldown_until_ms = optional_u64_field(event, "cooldownUntilMs")?;
        record.last_event_id = event.event_id.clone();
        self.touch("slots", event);
        let release = self.release_mut(event)?;
        release.standby_written = true;
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        Ok(())
    }

    fn release_cooldown(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let blocked = u64_field(event, "blockedAtMs")?;
        let cooldown = u64_field(event, "cooldownUntilMs")?;
        let release = self.release_mut(event)?;
        release.final_status = Some("cooldown_blocked".to_string());
        release.finalized_at_ms = Some(blocked);
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        let slot_id = string_field(event, "slotId")?;
        let slot = self
            .state
            .slots
            .get_mut(&slot_id)
            .ok_or_else(|| ProjectionError::Invalid("slot missing".to_string()))?;
        slot.cooldown_until_ms = Some(cooldown);
        slot.last_event_id = event.event_id.clone();
        self.touch("slots", event);
        Ok(())
    }

    fn cooldown_cleared(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let id = string_field(event, "slotId")?;
        let slot = self
            .state
            .slots
            .get_mut(&id)
            .ok_or_else(|| ProjectionError::Invalid("slot missing".to_string()))?;
        slot.cooldown_until_ms = None;
        slot.allocatable = true;
        slot.last_event_id = event.event_id.clone();
        self.touch("slots", event);
        Ok(())
    }

    fn release_finalized(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let final_status = string_field(event, "finalStatus")?;
        let allocatable = bool_field(event, "allocatable")?;
        if allocatable != (final_status == "allocatable") {
            return invalid("release final allocatable");
        }
        let release_id = string_field(event, "releaseId")?;
        let (subject_kind, subject_id) = {
            let release = self
                .state
                .releases
                .get(&release_id)
                .ok_or_else(|| ProjectionError::Invalid("release missing".to_string()))?;
            (release.subject_kind.clone(), release.subject_id.clone())
        };
        let release = self
            .state
            .releases
            .get_mut(&release_id)
            .expect("known release");
        if release.final_status.is_some()
            && !(release.final_status.as_deref() == Some("cooldown_blocked")
                && final_status == "allocatable")
        {
            return invalid("release final status replacement");
        }
        if release.runtime_outcome == "failed" && (final_status != "cleanup_failed" || allocatable)
        {
            return invalid("runtime stop failure requires cleanup_failed");
        }
        release.final_status = Some(final_status);
        release.finalized_at_ms = Some(u64_field(event, "finalizedAtMs")?);
        release.last_event_id = event.event_id.clone();
        self.touch("releases", event);
        if subject_kind == "request" {
            let request =
                self.state.requests.get_mut(&subject_id).ok_or_else(|| {
                    ProjectionError::Invalid("release request missing".to_string())
                })?;
            request.state = "released".to_string();
            request.last_event_id = event.event_id.clone();
            self.touch("requests", event);
        }
        Ok(())
    }

    fn qa_matrix(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let mut record = self.qa_record(event)?;
        counters::record_matrix(
            &mut record,
            u8_field(event, "matrixIteration")?,
            &string_field(event, "sourceFingerprint")?,
            u8_field(event, "casesPassed")?,
            u8_field(event, "casesTotal")?,
            &event.event_id,
        )
        .map_err(|error| ProjectionError::Invalid(error.to_string()))?;
        self.state.qa_counters.insert("qa".to_string(), record);
        self.touch("qa_counters", event);
        Ok(())
    }

    fn qa_repeat(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let mut record = self.qa_record(event)?;
        counters::record_repeat(
            &mut record,
            &string_field(event, "caseId")?,
            u8_field(event, "repetitionIndex")?,
            &string_field(event, "sourceFingerprint")?,
            bool_field(event, "passed")?,
            &event.event_id,
        )
        .map_err(|error| ProjectionError::Invalid(error.to_string()))?;
        self.state.qa_counters.insert("qa".to_string(), record);
        self.touch("qa_counters", event);
        Ok(())
    }

    fn qa_reset(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        let mut record = self.qa_record(event)?;
        let fingerprint = string_field(event, "sourceFingerprint")?;
        match string_field(event, "scope")?.as_str() {
            "all" => counters::reset_all(&mut record, &fingerprint, &event.event_id),
            "case" => counters::reset_case(
                &mut record,
                &optional_string_field(event, "caseId")?.ok_or_else(|| {
                    ProjectionError::Invalid("case reset without caseId".to_string())
                })?,
                &fingerprint,
                &event.event_id,
            ),
            _ => return invalid("QA reset scope"),
        }
        .map_err(|error| ProjectionError::Invalid(error.to_string()))?;
        self.state.qa_counters.insert("qa".to_string(), record);
        self.touch("qa_counters", event);
        Ok(())
    }

    fn qa_record(&self, event: &EventEnvelope) -> Result<QaCounterRecord, ProjectionError> {
        self.state
            .qa_counters
            .get("qa")
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                counters::empty(event.event_id.clone())
                    .map_err(|error| ProjectionError::Invalid(error.to_string()))
            })
    }

    fn release_mut(
        &mut self,
        event: &EventEnvelope,
    ) -> Result<&mut ReleaseRecord, ProjectionError> {
        let id = string_field(event, "releaseId")?;
        self.state
            .releases
            .get_mut(&id)
            .ok_or_else(|| ProjectionError::Invalid("release missing".to_string()))
    }
}

fn cas_grant(
    event: &EventEnvelope,
    id: String,
    kind: &str,
    subject_id: String,
    generation: u16,
    time_field: &str,
) -> Result<CasRecord, ProjectionError> {
    let granted = u64_field(event, time_field)?;
    let renew = u64_field(event, "renewAtMs")?;
    let expires = u64_field(event, "expiresAtMs")?;
    if generation == 0
        || renew != granted.checked_add(100_000).unwrap_or(0)
        || expires != granted.checked_add(300_000).unwrap_or(0)
    {
        return invalid("CAS grant timing/generation");
    }
    let fencing = string_field(event, "fencingTokenSha256")?;
    validate_h256(&fencing).map_err(|error| ProjectionError::Invalid(error.to_string()))?;
    Ok(CasRecord {
        id,
        kind: kind.to_string(),
        subject_id,
        owner: event.writer.clone(),
        generation,
        renewal_revision: 1,
        fencing_token_sha256: Some(fencing),
        granted_at_ms: granted,
        renew_at_ms: renew,
        expires_at_ms: expires,
        status: "active".to_string(),
        released_at_ms: None,
        release_event_id: None,
        last_event_id: event.event_id.clone(),
    })
}

fn renew_cas(
    record: &mut CasRecord,
    event: &EventEnvelope,
    generation_field: &str,
    renewed_field: &str,
) -> Result<(), ProjectionError> {
    let renewed = u64_field(event, renewed_field)?;
    let revision = u16_field(event, "renewalRevision")?;
    if record.status != "active"
        || u16_field(event, generation_field)? != record.generation
        || revision != record.renewal_revision.checked_add(1).unwrap_or(0)
        || renewed >= record.expires_at_ms
        || u64_field(event, "renewAtMs")? != renewed.checked_add(100_000).unwrap_or(0)
        || u64_field(event, "expiresAtMs")? != renewed.checked_add(300_000).unwrap_or(0)
    {
        return invalid("CAS renewal");
    }
    record.renewal_revision = revision;
    record.renew_at_ms = u64_field(event, "renewAtMs")?;
    record.expires_at_ms = u64_field(event, "expiresAtMs")?;
    record.last_event_id = event.event_id.clone();
    Ok(())
}

fn release_cas(
    record: &mut CasRecord,
    event: &EventEnvelope,
    generation_field: &str,
    released_field: &str,
) -> Result<(), ProjectionError> {
    if record.status != "active" || u16_field(event, generation_field)? != record.generation {
        return invalid("CAS release");
    }
    record.status = "released".to_string();
    record.released_at_ms = Some(u64_field(event, released_field)?);
    record.release_event_id = Some(event.event_id.clone());
    record.last_event_id = event.event_id.clone();
    Ok(())
}

fn ensure_no_active(
    records: &BTreeMap<String, CasRecord>,
    subject_id: &str,
) -> Result<(), ProjectionError> {
    if records
        .values()
        .any(|record| record.subject_id == subject_id && record.status == "active")
    {
        invalid("active resource subject conflict")
    } else {
        Ok(())
    }
}

fn ensure_no_active_owners(
    records: &BTreeMap<String, RuntimeOwnerRecord>,
    subject_id: &str,
) -> Result<(), ProjectionError> {
    if records
        .values()
        .any(|record| record.cas.subject_id == subject_id && record.cas.status == "active")
    {
        invalid("active runtime owner subject conflict")
    } else {
        Ok(())
    }
}

fn object(event: &EventEnvelope) -> Result<&Map<String, Value>, ProjectionError> {
    event
        .payload
        .as_object()
        .ok_or_else(|| ProjectionError::Invalid("payload object".to_string()))
}

fn string_field(event: &EventEnvelope, key: &str) -> Result<String, ProjectionError> {
    object(event)?
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| ProjectionError::Invalid(format!("{key} string")))
}

fn optional_string_field(
    event: &EventEnvelope,
    key: &str,
) -> Result<Option<String>, ProjectionError> {
    match object(event)?.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => invalid(&format!("{key} optional string")),
    }
}

fn u64_field(event: &EventEnvelope, key: &str) -> Result<u64, ProjectionError> {
    object(event)?
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProjectionError::Invalid(format!("{key} integer")))
}

fn optional_u64_field(event: &EventEnvelope, key: &str) -> Result<Option<u64>, ProjectionError> {
    match object(event)?.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| ProjectionError::Invalid(format!("{key} optional integer"))),
        None => invalid(&format!("{key} missing")),
    }
}

fn u16_field(event: &EventEnvelope, key: &str) -> Result<u16, ProjectionError> {
    u16::try_from(u64_field(event, key)?)
        .map_err(|_| ProjectionError::Invalid(format!("{key} range")))
}

fn u8_field(event: &EventEnvelope, key: &str) -> Result<u8, ProjectionError> {
    u8::try_from(u64_field(event, key)?)
        .map_err(|_| ProjectionError::Invalid(format!("{key} range")))
}

fn bool_field(event: &EventEnvelope, key: &str) -> Result<bool, ProjectionError> {
    object(event)?
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| ProjectionError::Invalid(format!("{key} boolean")))
}

fn mode(applicable: bool) -> String {
    if applicable {
        "pending"
    } else {
        "not_applicable"
    }
    .to_string()
}

fn invalid<T>(message: &str) -> Result<T, ProjectionError> {
    Err(ProjectionError::Invalid(message.to_string()))
}

fn set_unique_source(
    target: &mut Option<String>,
    event_id: &str,
    error: &str,
) -> Result<(), ProjectionError> {
    if target.replace(event_id.to_string()).is_some() {
        return invalid(error);
    }
    Ok(())
}

struct ProjectionLock {
    path: PathBuf,
}

impl ProjectionLock {
    fn acquire(path: PathBuf) -> Result<Self, ProjectionError> {
        // ProjectionStore validates the complete parent chain before this atomic lock-leaf create.
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => Ok(Self { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(ProjectionError::LockContended)
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for ProjectionLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), ProjectionError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_private(path: &Path) -> Result<Vec<u8>, ProjectionError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ProjectionError::UnsafePath(path.to_path_buf()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
