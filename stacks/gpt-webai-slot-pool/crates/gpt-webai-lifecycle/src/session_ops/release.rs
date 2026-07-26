use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::claims::derived_id;
use crate::config::{now_ms, SupervisorConfig};
use crate::contracts::browser::{EvidenceMediaType, EvidenceRef};
use crate::contracts::cli::{CommandOutcome, CommandOutcomeError};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::h256;
use crate::contracts::projection::{CasRecord, ProjectionState, RuntimeOwnerRecord};
use crate::journal::canonical::canonical_bytes;
use crate::journal::{EventStore, HeadStore};
use crate::release::ownership::{authorize_stop, StopAuthorization, StopAuthorizationInput};
use crate::runtime::control::RuntimeReleaseMode;
use crate::runtime::docker_control::DockerControlError;
use crate::runtime::ownership::{process_absent, DeadOwnerProof};
use crate::runtime::{parse_docker_inspect, write_runtime_adoption_evidence};
use crate::sessions::{read_session_record, update_session_record, SessionRecord};
use crate::slots;

use super::journal::{NewEvent, SessionJournal, SessionJournalError};
use super::runtime_r13::{
    observe_owned_runtime, stop_owned_runtime, stop_runtime, AcquiredRuntime,
    SessionRuntimeR13Error,
};

mod partial;
pub use partial::{release_session_partial, SessionPartialReleaseInput};

#[derive(Debug, Error)]
pub enum SessionReleaseError {
    #[error("session release journal failed: {0}")]
    Journal(#[from] SessionJournalError),
    #[error("session release runtime stop failed: {0}")]
    Runtime(#[from] SessionRuntimeR13Error),
    #[error("session release id failed: {0}")]
    Id(#[from] crate::claims::CasError),
    #[error("session release evidence failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("session release evidence serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session release outcome failed: {0}")]
    Outcome(#[from] CommandOutcomeError),
    #[error("session release event store failed: {0}")]
    Store(#[from] crate::journal::EventStoreError),
    #[error("session release journal lock failed: {0}")]
    Head(#[from] crate::journal::HeadError),
    #[error("session release Docker inspection failed: {0}")]
    Docker(#[from] DockerControlError),
    #[error("session release ownership failed: {0}")]
    Ownership(#[from] crate::release::ReleaseError),
    #[error("session release runtime identity failed: {0}")]
    RuntimeIdentity(#[from] crate::runtime::ownership::RuntimeIdentityError),
    #[error("session release runtime evidence failed: {0}")]
    RuntimeEvidence(#[from] crate::runtime::RuntimeEvidenceError),
    #[error("session release persisted record failed: {0}")]
    Session(#[from] crate::sessions::SessionRecordError),
    #[error("session release source contract failed: {0}")]
    Contract(&'static str),
}

pub struct ExplicitReleaseInput {
    pub config: SupervisorConfig,
    pub operation_id: String,
    pub session_id: Option<String>,
    pub slot_id: Option<String>,
    pub fencing_token: Option<String>,
    pub docker_bin: PathBuf,
    pub runtime_stop_timeout: Duration,
}

#[derive(Clone)]
struct ActiveReleaseResources {
    request_claim: Option<CasRecord>,
    session_claim: Option<CasRecord>,
    lease: Option<CasRecord>,
    owner: Option<RuntimeOwnerRecord>,
}

struct ExplicitTarget {
    subject_kind: &'static str,
    subject_id: String,
    slot_id: String,
    session: Option<SessionRecord>,
}

pub fn execute_explicit_release(
    input: ExplicitReleaseInput,
) -> Result<CommandOutcome, SessionReleaseError> {
    drop(crate::provider_runner::ensure_private_state_root(
        &input.config.state_root,
    )?);
    let Some(target) = resolve_explicit_target(&input)? else {
        return explicit_outcome(
            &input,
            None,
            "release.target_unknown",
            "release target is not known to the R13 state store",
            Some("release.target_unknown"),
            &[],
            None,
        );
    };
    let request_id = target
        .session
        .as_ref()
        .and_then(|record| record.request_id.clone());
    let run_id = target
        .session
        .as_ref()
        .and_then(|record| record.run_id.clone());
    let mut journal = SessionJournal::open(
        &input.config,
        input.operation_id.clone(),
        request_id,
        run_id,
    )?;
    let guard = match HeadStore::new(&input.config.state_root).acquire_mutation() {
        Ok(guard) => guard,
        Err(crate::journal::HeadError::LockContended(_)) => {
            return explicit_outcome(
                &input,
                Some(&target),
                "release.lock_contended",
                "the lifecycle state-store mutation lock is contended",
                Some("lock.contended"),
                &[],
                None,
            );
        }
        Err(error) => return Err(error.into()),
    };
    let projection = journal.replay()?.state;
    let events = EventStore::new(&input.config.state_root).load_all()?;
    let resources = active_resources(&projection, &events, &target)?;
    let incomplete = projection
        .releases
        .values()
        .find(|release| {
            release.subject_kind == target.subject_kind
                && release.subject_id == target.subject_id
                && release.final_status.is_none()
        })
        .cloned();
    if let Some(release) = incomplete {
        drop(guard);
        return continue_explicit_release(input, target, journal, release.release_id);
    }
    if resources.is_empty() {
        let finalized = projection.releases.values().any(|release| {
            release.subject_kind == target.subject_kind
                && release.subject_id == target.subject_id
                && release.final_status.is_some()
        });
        drop(guard);
        let result_kind = if finalized {
            "release.already_released"
        } else {
            "release.target_unknown"
        };
        return explicit_outcome(
            &input,
            Some(&target),
            result_kind,
            if finalized {
                "release target has no active resources and is already finalized"
            } else {
                "release target has no active R13 resources"
            },
            (!finalized).then_some(result_kind),
            &[],
            None,
        );
    }
    if let (Some(owner), Some(token)) = (&resources.owner, input.fencing_token.as_deref()) {
        if !crate::runtime::ownership::current_owner_can_stop(
            owner,
            owner.cas.generation,
            token,
            now_ms(),
        ) {
            drop(guard);
            return explicit_outcome(
                &input,
                Some(&target),
                "release.fencing_mismatch",
                "the presented fencing token does not authorize the active runtime owner",
                Some("release.fencing_mismatch"),
                &[],
                resources.owner.as_ref(),
            );
        }
    }
    let release_id = derived_id(
        "release_",
        &json!([
            "pr72.release.r13.v1",
            target.subject_kind,
            target.subject_id,
            input.operation_id
        ]),
    )?;
    let started_at = now_ms();
    journal.append_with_guard(
        &guard,
        NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.clone(),
            event_type: EventType::ReleaseStarted,
            payload: json!({
                "releaseId":release_id,"subjectKind":target.subject_kind,
                "subjectId":target.subject_id,"reason":"release.explicit",
                "startedAtMs":started_at
            }),
            predecessor_event_id: None,
            source_event_ids: resources.event_ids(),
            created_at_ms: started_at,
        },
    )?;
    drop(guard);
    continue_explicit_release(input, target, journal, release_id)
}

fn continue_explicit_release(
    input: ExplicitReleaseInput,
    target: ExplicitTarget,
    mut journal: SessionJournal,
    release_id: String,
) -> Result<CommandOutcome, SessionReleaseError> {
    let evidence_root = release_evidence_root(
        &input.config,
        &explicit_request_key(&target, &input.operation_id),
        &input.operation_id,
    )?;
    let mut projection = journal.replay()?.state;
    let mut events = EventStore::new(&input.config.state_root).load_all()?;
    let mut release = projection
        .releases
        .get(&release_id)
        .cloned()
        .ok_or(SessionReleaseError::Contract("release projection missing"))?;
    let preserved = if let Some(event_id) = release.evidence_preserved_event_id.as_deref() {
        event_by_id(&events, event_id)?.clone()
    } else {
        let (path, sha256) = write_evidence_manifest(
            &input.config.state_root,
            &evidence_root,
            journal.event_ids(),
            &[],
        )?;
        let at = now_ms();
        let event = journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.clone(),
            event_type: EventType::ReleaseEvidencePreserved,
            payload: json!({
                "releaseId":release_id,"evidenceManifestPath":path,
                "evidenceManifestSha256":sha256,"preservedAtMs":at
            }),
            predecessor_event_id: Some(release.last_event_id.clone()),
            source_event_ids: vec![release.last_event_id.clone()],
            created_at_ms: at,
        })?;
        events.push(event.clone());
        event
    };

    projection = journal.replay()?.state;
    release = projection
        .releases
        .get(&release_id)
        .cloned()
        .ok_or(SessionReleaseError::Contract("release projection missing"))?;
    let mut resources = active_resources(&projection, &events, &target)?;
    let runtime_event = if release.runtime_outcome == "pending" {
        let event = execute_runtime_release(
            &input,
            &target,
            &mut journal,
            &release_id,
            &preserved,
            &resources,
            &evidence_root,
        )?;
        events.push(event.clone());
        event
    } else {
        latest_release_event(
            &events,
            &release_id,
            &[
                EventType::RuntimeStopped,
                EventType::RuntimeStopFailed,
                EventType::RuntimeStopSkipped,
            ],
        )?
        .clone()
    };

    projection = journal.replay()?.state;
    release = projection
        .releases
        .get(&release_id)
        .cloned()
        .ok_or(SessionReleaseError::Contract("release projection missing"))?;
    events = EventStore::new(&input.config.state_root).load_all()?;
    resources = active_resources(&projection, &events, &target)?;
    let committed =
        latest_release_event(&events, &release_id, &[EventType::ReleaseCleanupCommitted]).cloned();
    let committed = if let Ok(committed) = committed {
        committed
    } else {
        let cleanup =
            latest_release_event(&events, &release_id, &[EventType::ReleaseCleanupStarted])
                .cloned()
                .unwrap_or_else(|_| EventEnvelope::clone(&runtime_event));
        let cleanup = if cleanup.event_type == EventType::ReleaseCleanupStarted {
            cleanup
        } else {
            let at = now_ms();
            journal.append(NewEvent {
                aggregate_kind: AggregateKind::Release,
                aggregate_id: release_id.clone(),
                event_type: EventType::ReleaseCleanupStarted,
                payload: json!({"releaseId":release_id,"startedAtMs":at}),
                predecessor_event_id: Some(runtime_event.event_id.clone()),
                source_event_ids: vec![runtime_event.event_id.clone()],
                created_at_ms: at,
            })?
        };
        cleanup_resources(
            &mut journal,
            &release,
            &release_id,
            &cleanup,
            &runtime_event,
            &resources,
        )?
    };
    let final_result = finalize_explicit_release(
        &input,
        &target,
        &mut journal,
        &release_id,
        &committed,
        &runtime_event,
    )?;
    if let Some(mut record) = target.session.clone() {
        record.updated_at_ms = now_ms();
        update_session_record(&input.config.state_root, &record)?;
    }
    explicit_outcome(
        &input,
        Some(&target),
        final_result,
        "R13 evidence-first release completed",
        matches!(
            final_result,
            "release.stop_failed" | "release.cleanup_failed"
        )
        .then(|| final_result),
        journal.event_ids(),
        resources.owner.as_ref(),
    )
}

fn resolve_explicit_target(
    input: &ExplicitReleaseInput,
) -> Result<Option<ExplicitTarget>, SessionReleaseError> {
    if let Some(session_id) = &input.session_id {
        let session = match read_session_record(&input.config.state_root, session_id) {
            Ok(record) => record,
            Err(crate::sessions::SessionRecordError::Missing(_)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        return Ok(Some(ExplicitTarget {
            subject_kind: "session_operation",
            subject_id: session_id.clone(),
            slot_id: session.slot_id.clone(),
            session: Some(session),
        }));
    }
    let Some(slot_id) = input.slot_id.as_ref() else {
        return Err(SessionReleaseError::Contract("release target missing"));
    };
    if !slots::inventory(&input.config)
        .iter()
        .any(|slot| slot.slot_id.0 == *slot_id)
    {
        return Ok(None);
    }
    Ok(Some(ExplicitTarget {
        subject_kind: "slot",
        subject_id: slot_id.clone(),
        slot_id: slot_id.clone(),
        session: None,
    }))
}

fn active_resources(
    projection: &ProjectionState,
    events: &[EventEnvelope],
    target: &ExplicitTarget,
) -> Result<ActiveReleaseResources, SessionReleaseError> {
    let mut claim_roles = BTreeMap::new();
    for claim in projection.claims.values() {
        let grant = aggregate_event(
            events,
            &claim.id,
            &[
                EventType::RequestClaimGranted,
                EventType::SessionOperationClaimGranted,
            ],
        )?;
        let role = match grant.event_type {
            EventType::RequestClaimGranted => "request",
            EventType::SessionOperationClaimGranted => "session",
            _ => return Err(SessionReleaseError::Contract("claim role")),
        };
        claim_roles.insert(claim.id.clone(), role);
    }
    let mut lease_claims = BTreeMap::new();
    for lease in projection.leases.values() {
        let grant = aggregate_event(
            events,
            &lease.id,
            &[
                EventType::SlotLeaseGranted,
                EventType::PersistedSessionLeaseGranted,
            ],
        )?;
        let claim_id = grant
            .payload
            .get("claimId")
            .and_then(serde_json::Value::as_str)
            .ok_or(SessionReleaseError::Contract("lease claim link"))?;
        lease_claims.insert(lease.id.clone(), claim_id.to_string());
    }
    let mut owner_leases = BTreeMap::new();
    for owner in projection.runtime_owners.values() {
        let grant = aggregate_event(
            events,
            &owner.cas.id,
            &[
                EventType::RuntimeOwnershipGranted,
                EventType::RuntimeOwnershipAdopted,
                EventType::SessionRuntimeOwnershipGranted,
                EventType::SessionRuntimeOwnershipAdopted,
                EventType::RuntimeTakeoverProven,
            ],
        )?;
        let lease_id = owner_lease_id(events, grant)?;
        if let Some(lease_id) = lease_id {
            owner_leases.insert(owner.cas.id.clone(), lease_id);
        }
    }

    let mut related_claims = Vec::new();
    for claim in projection.claims.values() {
        let role = claim_roles.get(&claim.id).copied();
        if claim.subject_id == target.subject_id
            && matches!(
                (target.subject_kind, role),
                ("request", Some("request")) | ("session_operation", Some("session"))
            )
        {
            related_claims.push(claim.id.clone());
        }
    }
    let mut related_leases = Vec::new();
    for lease in projection.leases.values() {
        let direct_slot = target.subject_kind == "slot" && lease.subject_id == target.subject_id;
        let linked = lease_claims
            .get(&lease.id)
            .is_some_and(|claim| related_claims.contains(claim));
        if direct_slot || linked {
            related_leases.push(lease.id.clone());
            if let Some(claim) = lease_claims.get(&lease.id) {
                if !related_claims.contains(claim) {
                    related_claims.push(claim.clone());
                }
            }
        }
    }

    let mut result = ActiveReleaseResources {
        request_claim: None,
        session_claim: None,
        lease: None,
        owner: None,
    };
    for claim_id in related_claims {
        let Some(claim) = projection.claims.get(&claim_id) else {
            continue;
        };
        if claim.status != "active" {
            continue;
        }
        match claim_roles.get(&claim_id).copied() {
            Some("request") => set_unique(&mut result.request_claim, claim, "request claim")?,
            Some("session") => set_unique(&mut result.session_claim, claim, "session claim")?,
            _ => return Err(SessionReleaseError::Contract("claim role missing")),
        }
    }
    for lease_id in &related_leases {
        let Some(lease) = projection.leases.get(lease_id) else {
            continue;
        };
        if lease.status == "active" {
            set_unique(&mut result.lease, lease, "slot lease")?;
        }
    }
    for owner in projection.runtime_owners.values() {
        let direct_slot =
            target.subject_kind == "slot" && owner.cas.subject_id == target.subject_id;
        let linked = owner_leases
            .get(&owner.cas.id)
            .is_some_and(|lease| related_leases.contains(lease));
        if owner.cas.status == "active" && (direct_slot || linked) {
            set_unique(&mut result.owner, owner, "runtime owner")?;
        }
    }
    Ok(result)
}

fn owner_lease_id(
    events: &[EventEnvelope],
    grant: &EventEnvelope,
) -> Result<Option<String>, SessionReleaseError> {
    if let Some(lease_id) = grant
        .payload
        .get("leaseId")
        .and_then(serde_json::Value::as_str)
    {
        return Ok(Some(lease_id.to_string()));
    }
    if let Some(lease) = grant
        .source_event_ids
        .iter()
        .filter_map(|source| event_by_id(events, source).ok())
        .find(|source| {
            matches!(
                source.event_type,
                EventType::SlotLeaseGranted
                    | EventType::SlotLeaseRenewed
                    | EventType::PersistedSessionLeaseGranted
            )
        })
    {
        return Ok(Some(lease.aggregate.id.clone()));
    }
    if grant.event_type == EventType::RuntimeTakeoverProven {
        let prior_owner_id = grant
            .payload
            .get("priorOwnerId")
            .and_then(serde_json::Value::as_str)
            .ok_or(SessionReleaseError::Contract("takeover prior owner link"))?;
        let prior_grant = aggregate_event(
            events,
            prior_owner_id,
            &[
                EventType::RuntimeOwnershipGranted,
                EventType::RuntimeOwnershipAdopted,
                EventType::SessionRuntimeOwnershipGranted,
                EventType::SessionRuntimeOwnershipAdopted,
                EventType::RuntimeTakeoverProven,
            ],
        )?;
        return owner_lease_id(events, prior_grant);
    }
    Ok(None)
}

fn set_unique<T: Clone>(
    target: &mut Option<T>,
    value: &T,
    name: &'static str,
) -> Result<(), SessionReleaseError> {
    if target.replace(value.clone()).is_some() {
        return Err(SessionReleaseError::Contract(name));
    }
    Ok(())
}

impl ActiveReleaseResources {
    fn is_empty(&self) -> bool {
        self.request_claim.is_none()
            && self.session_claim.is_none()
            && self.lease.is_none()
            && self.owner.is_none()
    }

    fn event_ids(&self) -> Vec<String> {
        self.request_claim
            .iter()
            .map(|record| record.last_event_id.clone())
            .chain(
                self.session_claim
                    .iter()
                    .map(|record| record.last_event_id.clone()),
            )
            .chain(self.lease.iter().map(|record| record.last_event_id.clone()))
            .chain(
                self.owner
                    .iter()
                    .map(|record| record.cas.last_event_id.clone()),
            )
            .collect()
    }
}

fn aggregate_event<'a>(
    events: &'a [EventEnvelope],
    aggregate_id: &str,
    kinds: &[EventType],
) -> Result<&'a EventEnvelope, SessionReleaseError> {
    let mut matches = events
        .iter()
        .filter(|event| event.aggregate.id == aggregate_id && kinds.contains(&event.event_type));
    let event = matches
        .next()
        .ok_or(SessionReleaseError::Contract("aggregate origin missing"))?;
    if matches.next().is_some() {
        return Err(SessionReleaseError::Contract("aggregate origin duplicate"));
    }
    Ok(event)
}

fn event_by_id<'a>(
    events: &'a [EventEnvelope],
    event_id: &str,
) -> Result<&'a EventEnvelope, SessionReleaseError> {
    events
        .iter()
        .find(|event| event.event_id == event_id)
        .ok_or(SessionReleaseError::Contract("event missing"))
}

fn latest_release_event<'a>(
    events: &'a [EventEnvelope],
    release_id: &str,
    kinds: &[EventType],
) -> Result<&'a EventEnvelope, SessionReleaseError> {
    events
        .iter()
        .rev()
        .find(|event| event.aggregate.id == release_id && kinds.contains(&event.event_type))
        .ok_or(SessionReleaseError::Contract("release stage event missing"))
}

#[allow(clippy::too_many_arguments)]
fn execute_runtime_release(
    input: &ExplicitReleaseInput,
    target: &ExplicitTarget,
    journal: &mut SessionJournal,
    release_id: &str,
    preserved: &EventEnvelope,
    resources: &ActiveReleaseResources,
    evidence_root: &Path,
) -> Result<EventEnvelope, SessionReleaseError> {
    let Some(owner) = resources.owner.as_ref() else {
        let at = now_ms();
        return Ok(journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.to_string(),
            event_type: EventType::RuntimeStopSkipped,
            payload: json!({
                "releaseId":release_id,"runtimeOwnerId":null,
                "reason":"runtime.not_acquired","proofAttempt":null,"skippedAtMs":at
            }),
            predecessor_event_id: Some(preserved.event_id.clone()),
            source_event_ids: vec![preserved.event_id.clone()],
            created_at_ms: at,
        })?);
    };
    let proof = if input.fencing_token.is_none() {
        dead_owner_proof_attempt(input, target, owner, resources, evidence_root)?
    } else {
        None
    };
    let authorization = authorize_stop(StopAuthorizationInput {
        owner: Some(owner),
        presented_generation: input.fencing_token.as_ref().map(|_| owner.cas.generation),
        fencing_token: input.fencing_token.as_deref(),
        now_ms: now_ms(),
        dead_owner_proof: proof.as_ref(),
        release_id,
        takeover_writer: journal.writer().clone(),
        takeover_event_id: format!("evt_{}", "0".repeat(64)),
    });
    let authorization = match authorization {
        Ok(value) => value,
        Err(crate::release::ReleaseError::FencingMismatch) => {
            return Err(SessionReleaseError::Ownership(
                crate::release::ReleaseError::FencingMismatch,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let (stop_owner, takeover_event) = match authorization {
        StopAuthorization::CurrentOwner(owner) => (owner, None),
        StopAuthorization::Takeover { replacement, .. } => {
            let proof = proof
                .as_ref()
                .ok_or(SessionReleaseError::Contract("takeover proof missing"))?;
            let at = proof.proven_at_ms;
            let event = journal.append(NewEvent {
                aggregate_kind: AggregateKind::RuntimeOwner,
                aggregate_id: replacement.cas.id.clone(),
                event_type: EventType::RuntimeTakeoverProven,
                payload: json!({
                    "releaseId":release_id,"slotId":target.slot_id,
                    "priorOwnerId":owner.cas.id,"priorGeneration":owner.cas.generation,
                    "newOwnerId":replacement.cas.id,"newGeneration":replacement.cas.generation,
                    "deadOwnerProof":proof,"provenAtMs":at
                }),
                predecessor_event_id: None,
                source_event_ids: vec![owner.cas.last_event_id.clone(), preserved.event_id.clone()],
                created_at_ms: at,
            })?;
            ((*replacement).clone(), Some(event))
        }
        StopAuthorization::OwnerAliveOrUnknown {
            owner,
            proof_attempt,
        } => {
            let at = now_ms();
            return Ok(journal.append(NewEvent {
                aggregate_kind: AggregateKind::Release,
                aggregate_id: release_id.to_string(),
                event_type: EventType::RuntimeStopSkipped,
                payload: json!({
                    "releaseId":release_id,"runtimeOwnerId":owner.cas.id,
                    "reason":"runtime.owner_alive_or_unknown",
                    "proofAttempt":proof_attempt,"skippedAtMs":at
                }),
                predecessor_event_id: Some(preserved.event_id.clone()),
                source_event_ids: vec![preserved.event_id.clone()],
                created_at_ms: at,
            })?);
        }
        StopAuthorization::NotAcquired => {
            return Err(SessionReleaseError::Contract(
                "runtime authorization lost owner",
            ));
        }
    };
    let started_at = now_ms();
    let stop_started = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.to_string(),
        event_type: EventType::RuntimeStopStarted,
        payload: json!({
            "releaseId":release_id,"runtimeOwnerId":stop_owner.cas.id,
            "ownerGeneration":stop_owner.cas.generation,
            "stopTimeoutMs":input.runtime_stop_timeout.as_millis() as u64,
            "startedAtMs":started_at
        }),
        predecessor_event_id: Some(preserved.event_id.clone()),
        source_event_ids: takeover_event
            .iter()
            .map(|event| event.event_id.clone())
            .collect(),
        created_at_ms: started_at,
    })?;
    let stop_root = if takeover_event.is_some() {
        let path = evidence_root.join("stop");
        crate::provider_runner::create_private_directory(&input.config.state_root, &path)?;
        path
    } else {
        evidence_root.to_path_buf()
    };
    let at = now_ms();
    match stop_owned_runtime(
        &input.config,
        &target.slot_id,
        &stop_root,
        &input.docker_bin,
        input.runtime_stop_timeout,
        at,
        &crate::runtime::RuntimeReceiptLabels {
            owner_id: stop_owner.cas.id.clone(),
            owner_generation: stop_owner.cas.generation,
            runtime_incarnation_id: stop_owner.runtime_incarnation_id.clone(),
        },
    ) {
        Ok(receipt) => Ok(journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.to_string(),
            event_type: EventType::RuntimeStopped,
            payload: json!({
                "releaseId":release_id,"runtimeOwnerId":stop_owner.cas.id,
                "ownerGeneration":stop_owner.cas.generation,"dockerStatus":"exited",
                "stopReceipt":receipt,"stoppedAtMs":at
            }),
            predecessor_event_id: Some(stop_started.event_id.clone()),
            source_event_ids: vec![stop_started.event_id],
            created_at_ms: at,
        })?),
        Err(error) => {
            let failure = write_runtime_failure_evidence(
                &input.config.state_root,
                &stop_root,
                &error.to_string(),
            )?;
            Ok(journal.append(NewEvent {
                aggregate_kind: AggregateKind::Release,
                aggregate_id: release_id.to_string(),
                event_type: EventType::RuntimeStopFailed,
                payload: json!({
                    "releaseId":release_id,"runtimeOwnerId":stop_owner.cas.id,
                    "ownerGeneration":stop_owner.cas.generation,"dockerStatus":"unknown",
                    "failureReceipt":failure,"reason":"runtime.stop_failed","failedAtMs":at
                }),
                predecessor_event_id: Some(stop_started.event_id.clone()),
                source_event_ids: vec![stop_started.event_id],
                created_at_ms: at,
            })?)
        }
    }
}

fn dead_owner_proof_attempt(
    input: &ExplicitReleaseInput,
    target: &ExplicitTarget,
    owner: &RuntimeOwnerRecord,
    resources: &ActiveReleaseResources,
    evidence_root: &Path,
) -> Result<Option<DeadOwnerProof>, SessionReleaseError> {
    let now = now_ms();
    if now < owner.cas.granted_at_ms
        || now
            < owner
                .cas
                .expires_at_ms
                .saturating_add(crate::runtime::ownership::TAKEOVER_GRACE_MS)
    {
        return Ok(None);
    }
    let labels = crate::runtime::RuntimeReceiptLabels {
        owner_id: owner.cas.id.clone(),
        owner_generation: owner.cas.generation,
        runtime_incarnation_id: owner.runtime_incarnation_id.clone(),
    };
    let inspect_bytes = match observe_owned_runtime(
        &input.config,
        &target.slot_id,
        &input.docker_bin,
        input.runtime_stop_timeout,
        now,
        &labels,
    ) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let inspect = parse_docker_inspect(&inspect_bytes)?;
    let takeover_root = evidence_root.join("takeover");
    crate::provider_runner::create_private_directory(&input.config.state_root, &takeover_root)?;
    let evidence = write_runtime_adoption_evidence(
        &input.config.state_root,
        &takeover_root,
        &target.slot_id,
        &inspect_bytes,
        now,
    )?;
    let claim_inactive = resources
        .request_claim
        .iter()
        .chain(resources.session_claim.iter())
        .all(|claim| cas_logically_inactive(claim, now));
    let lease_inactive = resources
        .lease
        .as_ref()
        .is_none_or(|lease| cas_logically_inactive(lease, now));
    let process_absent =
        process_absent(&owner.cas.owner, &journal_host_id(input)?).unwrap_or(false);
    Ok(Some(DeadOwnerProof {
        prior_owner_id: owner.cas.id.clone(),
        prior_generation: owner.cas.generation,
        expired_at_ms: owner.cas.expires_at_ms,
        grace_satisfied_at_ms: now,
        process_absent,
        container_label_owner_id: inspect.label_owner_id,
        container_label_generation: inspect.label_generation,
        lease_inactive,
        claim_inactive,
        evidence_refs: vec![evidence],
        proven_at_ms: now,
    }))
}

fn cas_logically_inactive(record: &CasRecord, now: u64) -> bool {
    record.status != "active" || (now >= record.granted_at_ms && now >= record.expires_at_ms)
}

fn journal_host_id(input: &ExplicitReleaseInput) -> Result<String, SessionReleaseError> {
    let journal = SessionJournal::open(&input.config, input.operation_id.clone(), None, None)?;
    Ok(journal.writer().host_id.clone())
}

fn cleanup_resources(
    journal: &mut SessionJournal,
    release: &crate::contracts::projection::ReleaseRecord,
    release_id: &str,
    cleanup: &EventEnvelope,
    runtime_event: &EventEnvelope,
    resources: &ActiveReleaseResources,
) -> Result<EventEnvelope, SessionReleaseError> {
    let preserve_foreign_owner = runtime_event.event_type == EventType::RuntimeStopSkipped
        && runtime_event
            .payload
            .get("reason")
            .and_then(serde_json::Value::as_str)
            == Some("runtime.owner_alive_or_unknown");
    let session_claim = if release.session_claim_release == "pending" {
        resources
            .session_claim
            .as_ref()
            .map(|record| append_claim_release(journal, record, release_id, cleanup, true, None))
            .transpose()?
    } else {
        None
    };
    let request_claim = if release.request_claim_release == "pending" {
        resources
            .request_claim
            .as_ref()
            .map(|record| {
                append_claim_release(
                    journal,
                    record,
                    release_id,
                    cleanup,
                    false,
                    session_claim.as_ref(),
                )
            })
            .transpose()?
    } else {
        None
    };
    let lease = if release.slot_lease_release == "pending" {
        let record = resources
            .lease
            .as_ref()
            .ok_or(SessionReleaseError::Contract("pending lease missing"))?;
        let at = now_ms();
        let mut sources = Vec::new();
        if let Some(event) = request_claim.as_ref().or(session_claim.as_ref()) {
            sources.push(event.event_id.clone());
        } else {
            sources.push(cleanup.event_id.clone());
        }
        Some(journal.append(NewEvent {
            aggregate_kind: AggregateKind::Lease,
            aggregate_id: record.id.clone(),
            event_type: EventType::SlotLeaseReleased,
            payload: json!({
                "leaseId":record.id,"leaseGeneration":record.generation,
                "releaseId":release_id,"releasedAtMs":at
            }),
            predecessor_event_id: Some(record.last_event_id.clone()),
            source_event_ids: sources,
            created_at_ms: at,
        })?)
    } else {
        None
    };
    let owner = if release.runtime_owner_release == "pending" && !preserve_foreign_owner {
        let record = resources
            .owner
            .as_ref()
            .ok_or(SessionReleaseError::Contract("pending owner missing"))?;
        let at = now_ms();
        let mut sources = Vec::new();
        if let Some(lease) = &lease {
            sources.push(lease.event_id.clone());
        }
        sources.push(runtime_event.event_id.clone());
        Some(journal.append(NewEvent {
            aggregate_kind: AggregateKind::RuntimeOwner,
            aggregate_id: record.cas.id.clone(),
            event_type: EventType::RuntimeOwnershipReleased,
            payload: json!({
                "runtimeOwnerId":record.cas.id,"ownerGeneration":record.cas.generation,
                "releaseId":release_id,
                "runtimeOutcome":runtime_outcome_literal(runtime_event.event_type),
                "releasedAtMs":at
            }),
            predecessor_event_id: Some(record.cas.last_event_id.clone()),
            source_event_ids: sources,
            created_at_ms: at,
        })?)
    } else {
        None
    };
    let at = now_ms();
    let sources = request_claim
        .iter()
        .chain(session_claim.iter())
        .chain(lease.iter())
        .chain(owner.iter())
        .map(|event| event.event_id.clone())
        .collect();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.to_string(),
        event_type: EventType::ReleaseCleanupCommitted,
        payload: json!({
            "releaseId":release_id,
            "requestClaimReleaseMode":if release.request_claim_release=="pending"{"released"}else{"not_applicable"},
            "sessionClaimReleaseMode":if release.session_claim_release=="pending"{"released"}else{"not_applicable"},
            "leaseReleaseMode":if release.slot_lease_release=="pending"{"released"}else{"not_applicable"},
            "ownerReleaseMode":if release.runtime_owner_release=="pending" && !preserve_foreign_owner{"released"}else{"not_applicable"},
            "committedAtMs":at
        }),
        predecessor_event_id: Some(cleanup.event_id.clone()),
        source_event_ids: sources,
        created_at_ms: at,
    })?)
}

fn append_claim_release(
    journal: &mut SessionJournal,
    record: &CasRecord,
    release_id: &str,
    cleanup: &EventEnvelope,
    session: bool,
    session_release: Option<&EventEnvelope>,
) -> Result<EventEnvelope, SessionReleaseError> {
    let at = now_ms();
    let mut sources = vec![cleanup.event_id.clone()];
    if let Some(event) = session_release {
        sources.push(event.event_id.clone());
    }
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Claim,
        aggregate_id: record.id.clone(),
        event_type: if session {
            EventType::SessionOperationClaimReleased
        } else {
            EventType::RequestClaimReleased
        },
        payload: json!({
            "claimId":record.id,"claimGeneration":record.generation,
            "releaseId":release_id,"releasedAtMs":at
        }),
        predecessor_event_id: Some(record.last_event_id.clone()),
        source_event_ids: sources,
        created_at_ms: at,
    })?)
}

fn runtime_outcome_literal(event_type: EventType) -> &'static str {
    match event_type {
        EventType::RuntimeStopped => "stopped",
        EventType::RuntimeStopFailed => "failed",
        EventType::RuntimeStopSkipped => "skipped",
        _ => "failed",
    }
}

fn finalize_explicit_release(
    input: &ExplicitReleaseInput,
    target: &ExplicitTarget,
    journal: &mut SessionJournal,
    release_id: &str,
    committed: &EventEnvelope,
    runtime_event: &EventEnvelope,
) -> Result<&'static str, SessionReleaseError> {
    let projection = journal.replay()?.state;
    let release = projection
        .releases
        .get(release_id)
        .ok_or(SessionReleaseError::Contract("release projection missing"))?;
    if let Some(status) = release.final_status.as_deref() {
        return Ok(result_for_final_status(status, runtime_event.event_type));
    }
    let slot = projection.slots.get(&target.slot_id);
    let has_lease = release.slot_lease_release != "not_applicable";
    if !has_lease {
        let at = now_ms();
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.to_string(),
            event_type: EventType::ReleaseFinalized,
            payload: json!({
                "releaseId":release_id,"finalStatus":"resources_released_no_slot",
                "allocatable":false,"finalizedAtMs":at
            }),
            predecessor_event_id: Some(committed.event_id.clone()),
            source_event_ids: Vec::new(),
            created_at_ms: at,
        })?;
        return Ok("release.allocatable");
    }
    let slot = slot.ok_or(SessionReleaseError::Contract(
        "release slot projection missing",
    ))?;
    let now = now_ms();
    let cooldown = slot.cooldown_until_ms.filter(|until| *until > now);
    let owner_alive_skip = runtime_event.event_type == EventType::RuntimeStopSkipped
        && runtime_event
            .payload
            .get("reason")
            .and_then(serde_json::Value::as_str)
            == Some("runtime.owner_alive_or_unknown");
    let (allocatable, final_status, result_kind) = match runtime_event.event_type {
        EventType::RuntimeStopFailed => (false, "cleanup_failed", "release.stop_failed"),
        EventType::RuntimeStopSkipped if owner_alive_skip => (
            false,
            "stop_skipped_owner_alive",
            "release.stop_skipped_owner_alive",
        ),
        _ if cooldown.is_some() => (false, "cooldown_blocked", "release.cooldown_blocked"),
        _ => (true, "allocatable", "release.allocatable"),
    };
    let standby = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: target.slot_id.clone(),
        event_type: EventType::SlotStandbyWritten,
        payload: json!({
            "slotId":target.slot_id,"releaseId":release_id,"allocatable":allocatable,
            "cooldownUntilMs":cooldown,"writtenAtMs":now
        }),
        predecessor_event_id: Some(slot.last_event_id.clone()),
        source_event_ids: vec![committed.event_id.clone()],
        created_at_ms: now,
    })?;
    if final_status == "cooldown_blocked" {
        let until = cooldown.ok_or(SessionReleaseError::Contract("cooldown missing"))?;
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.to_string(),
            event_type: EventType::ReleaseCooldownBlocked,
            payload: json!({
                "releaseId":release_id,"slotId":target.slot_id,
                "cooldownUntilMs":until,"blockedAtMs":now
            }),
            predecessor_event_id: Some(committed.event_id.clone()),
            source_event_ids: vec![standby.event_id],
            created_at_ms: now,
        })?;
    } else {
        journal.append(NewEvent {
            aggregate_kind: AggregateKind::Release,
            aggregate_id: release_id.to_string(),
            event_type: EventType::ReleaseFinalized,
            payload: json!({
                "releaseId":release_id,"finalStatus":final_status,
                "allocatable":allocatable,"finalizedAtMs":now
            }),
            predecessor_event_id: Some(committed.event_id.clone()),
            source_event_ids: vec![standby.event_id],
            created_at_ms: now,
        })?;
    }
    let _ = input;
    Ok(result_kind)
}

fn result_for_final_status(status: &str, runtime_event_type: EventType) -> &'static str {
    match status {
        "allocatable" | "resources_released_no_slot" => "release.allocatable",
        "cooldown_blocked" => "release.cooldown_blocked",
        "stop_skipped_owner_alive" => "release.stop_skipped_owner_alive",
        "cleanup_failed" if runtime_event_type == EventType::RuntimeStopFailed => {
            "release.stop_failed"
        }
        "cleanup_failed" => "release.cleanup_failed",
        _ => "release.cleanup_failed",
    }
}

fn explicit_outcome(
    input: &ExplicitReleaseInput,
    target: Option<&ExplicitTarget>,
    result_kind: &str,
    message: &str,
    reason: Option<&str>,
    event_ids: &[String],
    owner: Option<&RuntimeOwnerRecord>,
) -> Result<CommandOutcome, SessionReleaseError> {
    let mut outcome = CommandOutcome::select(
        "release",
        input.operation_id.clone(),
        result_kind,
        message,
        reason.map(str::to_string),
    )?;
    outcome.envelope.session_id = target
        .and_then(|value| value.session.as_ref())
        .map(|record| record.session_id.clone())
        .or_else(|| input.session_id.clone());
    outcome.envelope.slot_id = target
        .map(|value| value.slot_id.clone())
        .or_else(|| input.slot_id.clone());
    outcome.envelope.event_ids = event_ids.to_vec();
    outcome.envelope.runtime_owner_id = owner.map(|record| record.cas.id.clone());
    Ok(outcome)
}

fn explicit_request_key(target: &ExplicitTarget, operation_id: &str) -> String {
    match target.session.as_ref() {
        Some(record) => record
            .request_id
            .as_ref()
            .map(|request_id| format!("r-{request_id}"))
            .unwrap_or_else(|| format!("s-{}", record.session_id)),
        None => format!("d-{operation_id}"),
    }
}

fn write_runtime_failure_evidence(
    state_root: &Path,
    evidence_root: &Path,
    message: &str,
) -> Result<EvidenceRef, SessionReleaseError> {
    crate::provider_runner::create_private_directory(state_root, evidence_root)?;
    let target = evidence_root.join("runtime-stop.failure.md");
    let mut bytes = message
        .as_bytes()
        .iter()
        .copied()
        .take(4_096)
        .collect::<Vec<_>>();
    bytes.push(b'\n');
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            File::open(evidence_root)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(&target)? != bytes {
                return Err(error.into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    let relative = target
        .strip_prefix(state_root)
        .map_err(std::io::Error::other)?
        .to_str()
        .ok_or_else(|| std::io::Error::other("non-UTF8 evidence path"))?
        .replace('\\', "/");
    Ok(EvidenceRef {
        path: relative,
        sha256: h256(&bytes),
        size_bytes: bytes.len() as u64,
        media_type: EvidenceMediaType::Markdown,
    })
}

pub struct SessionReleaseInput<'a> {
    pub config: &'a SupervisorConfig,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub session_id: &'a str,
    pub slot_id: &'a str,
    pub claim_event: &'a EventEnvelope,
    pub lease_event: &'a EventEnvelope,
    pub owner_event: &'a EventEnvelope,
    pub source_event: &'a EventEnvelope,
    pub slot_predecessor: &'a EventEnvelope,
    pub acquired_runtime: &'a AcquiredRuntime,
    pub runtime_release_mode: &'a RuntimeReleaseMode,
    pub receipt_ids: &'a [String],
}

pub struct RequestReleaseInput<'a> {
    pub config: &'a SupervisorConfig,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub request_id: &'a str,
    pub slot_id: &'a str,
    pub request_claim_event: &'a EventEnvelope,
    pub session_claim_event: Option<&'a EventEnvelope>,
    pub lease_event: &'a EventEnvelope,
    pub owner_event: &'a EventEnvelope,
    pub source_event: &'a EventEnvelope,
    pub slot_predecessor: &'a EventEnvelope,
    pub acquired_runtime: &'a AcquiredRuntime,
    pub runtime_release_mode: &'a RuntimeReleaseMode,
    pub receipt_ids: &'a [String],
}

pub struct RequestClaimOnlyReleaseInput<'a> {
    pub config: &'a SupervisorConfig,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub request_id: &'a str,
    pub request_claim_event: &'a EventEnvelope,
    pub source_event: &'a EventEnvelope,
    pub receipt_ids: &'a [String],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseCompletion {
    pub stop_failed: bool,
}

struct ResourceReleaseInput<'a> {
    config: &'a SupervisorConfig,
    operation_id: &'a str,
    request_key: &'a str,
    subject_kind: &'static str,
    subject_id: &'a str,
    reason: &'static str,
    slot_id: &'a str,
    request_claim_event: Option<&'a EventEnvelope>,
    session_claim_event: Option<&'a EventEnvelope>,
    lease_event: &'a EventEnvelope,
    owner_event: &'a EventEnvelope,
    source_event: &'a EventEnvelope,
    slot_predecessor: &'a EventEnvelope,
    acquired_runtime: &'a AcquiredRuntime,
    runtime_release_mode: &'a RuntimeReleaseMode,
    receipt_ids: &'a [String],
}

pub fn release_session_resources(
    journal: &mut SessionJournal,
    input: SessionReleaseInput<'_>,
) -> Result<ReleaseCompletion, SessionReleaseError> {
    let reason = release_reason(input.source_event.event_type)?;
    release_resources(
        journal,
        ResourceReleaseInput {
            config: input.config,
            operation_id: input.operation_id,
            request_key: input.request_key,
            subject_kind: "session_operation",
            subject_id: input.session_id,
            reason,
            slot_id: input.slot_id,
            request_claim_event: None,
            session_claim_event: Some(input.claim_event),
            lease_event: input.lease_event,
            owner_event: input.owner_event,
            source_event: input.source_event,
            slot_predecessor: input.slot_predecessor,
            acquired_runtime: input.acquired_runtime,
            runtime_release_mode: input.runtime_release_mode,
            receipt_ids: input.receipt_ids,
        },
    )
}

pub fn release_request_resources(
    journal: &mut SessionJournal,
    input: RequestReleaseInput<'_>,
) -> Result<ReleaseCompletion, SessionReleaseError> {
    let reason = release_reason(input.source_event.event_type)?;
    release_resources(
        journal,
        ResourceReleaseInput {
            config: input.config,
            operation_id: input.operation_id,
            request_key: input.request_key,
            subject_kind: "request",
            subject_id: input.request_id,
            reason,
            slot_id: input.slot_id,
            request_claim_event: Some(input.request_claim_event),
            session_claim_event: input.session_claim_event,
            lease_event: input.lease_event,
            owner_event: input.owner_event,
            source_event: input.source_event,
            slot_predecessor: input.slot_predecessor,
            acquired_runtime: input.acquired_runtime,
            runtime_release_mode: input.runtime_release_mode,
            receipt_ids: input.receipt_ids,
        },
    )
}

pub fn release_request_claim_only(
    journal: &mut SessionJournal,
    input: RequestClaimOnlyReleaseInput<'_>,
) -> Result<ReleaseCompletion, SessionReleaseError> {
    if input.source_event.event_type != EventType::AllocationExhausted {
        return Err(SessionReleaseError::Contract(
            "claim-only release source must be AllocationExhausted",
        ));
    }
    let release_id = derived_id(
        "release_",
        &json!([
            "pr72.release.r13.v1",
            "request",
            input.request_id,
            input.operation_id
        ]),
    )?;
    let started_at = now_ms();
    let release = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseStarted,
        payload: json!({
            "releaseId":release_id,"subjectKind":"request","subjectId":input.request_id,
            "reason":"release.allocation_exhausted","startedAtMs":started_at
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
    let request_claim_released = release_request_claim(
        journal,
        input.request_claim_event,
        &release_id,
        &cleanup,
        None,
    )?;
    let committed_at = now_ms();
    let committed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseCleanupCommitted,
        payload: json!({
            "releaseId":release_id,"requestClaimReleaseMode":"released",
            "sessionClaimReleaseMode":"not_applicable","leaseReleaseMode":"not_applicable",
            "ownerReleaseMode":"not_applicable","committedAtMs":committed_at
        }),
        predecessor_event_id: Some(cleanup.event_id),
        source_event_ids: vec![request_claim_released.event_id],
        created_at_ms: committed_at,
    })?;
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
    Ok(ReleaseCompletion { stop_failed: false })
}

fn release_reason(event_type: EventType) -> Result<&'static str, SessionReleaseError> {
    match event_type {
        EventType::OutputPublished => Ok("release.output_published"),
        EventType::ArtifactClaimCompleted | EventType::ArtifactClaimFailed => {
            Ok("release.artifact_terminal")
        }
        EventType::PollFailed => Ok("release.poll_failed"),
        EventType::UploadFailed => Ok("release.upload_failed"),
        EventType::SendUncertain => Ok("release.send_uncertain"),
        EventType::SendFailed => Ok("release.send_failed"),
        EventType::ModelSelectionFailed => Ok("release.model_failed"),
        EventType::RootCaptureFailed => Ok("release.capture_failed"),
        EventType::SessionOperationFailed => Ok("release.session_operation_failed"),
        EventType::AllocationExhausted => Ok("release.allocation_exhausted"),
        EventType::SlotHealthObserved => Ok("release.readiness_failed"),
        EventType::PollProgress | EventType::SessionHydrated => {
            Ok("release.nonterminal_publication")
        }
        EventType::OutputPublishFailed => Ok("release.output_publish_failed"),
        _ => Err(SessionReleaseError::Contract("unsupported release source")),
    }
}

fn release_resources(
    journal: &mut SessionJournal,
    input: ResourceReleaseInput<'_>,
) -> Result<ReleaseCompletion, SessionReleaseError> {
    let release_id = derived_id(
        "release_",
        &json!([
            "pr72.release.r13.v1",
            input.subject_kind,
            input.subject_id,
            input.operation_id
        ]),
    )?;
    let started = now_ms();
    let release = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseStarted,
        payload: json!({
            "releaseId":release_id,"subjectKind":input.subject_kind,
            "subjectId":input.subject_id,"reason":input.reason,
            "startedAtMs":started
        }),
        predecessor_event_id: None,
        source_event_ids: vec![input.source_event.event_id.clone()],
        created_at_ms: started,
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
        source_event_ids: vec![release.event_id.clone()],
        created_at_ms: preserved_at,
    })?;

    let stop_started_at = now_ms();
    let stop_started = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::RuntimeStopStarted,
        payload: json!({
            "releaseId":release_id,"runtimeOwnerId":input.acquired_runtime.owner_id,
            "ownerGeneration":input.acquired_runtime.owner_generation,
            "stopTimeoutMs":super::runtime_r13::runtime_stop_timeout(input.runtime_release_mode).as_millis() as u64,
            "startedAtMs":stop_started_at
        }),
        predecessor_event_id: Some(preserved.event_id.clone()),
        source_event_ids: Vec::new(),
        created_at_ms: stop_started_at,
    })?;
    let stopped_at = now_ms();
    let (stopped, stop_failed) = match stop_runtime(
        input.config,
        input.slot_id,
        &evidence_root,
        input.acquired_runtime,
        input.runtime_release_mode,
        stopped_at,
    ) {
        Ok(stop_receipt) => (
            journal.append(NewEvent {
                aggregate_kind: AggregateKind::Release,
                aggregate_id: release_id.clone(),
                event_type: EventType::RuntimeStopped,
                payload: json!({
                    "releaseId":release_id,"runtimeOwnerId":input.acquired_runtime.owner_id,
                    "ownerGeneration":input.acquired_runtime.owner_generation,
                    "dockerStatus":"exited","stopReceipt":stop_receipt,"stoppedAtMs":stopped_at
                }),
                predecessor_event_id: Some(stop_started.event_id.clone()),
                source_event_ids: vec![stop_started.event_id.clone()],
                created_at_ms: stopped_at,
            })?,
            false,
        ),
        Err(error) => {
            let failure = write_runtime_failure_evidence(
                &input.config.state_root,
                &evidence_root,
                &error.to_string(),
            )?;
            (
                journal.append(NewEvent {
                    aggregate_kind: AggregateKind::Release,
                    aggregate_id: release_id.clone(),
                    event_type: EventType::RuntimeStopFailed,
                    payload: json!({
                        "releaseId":release_id,"runtimeOwnerId":input.acquired_runtime.owner_id,
                        "ownerGeneration":input.acquired_runtime.owner_generation,
                        "dockerStatus":"unknown","failureReceipt":failure,
                        "reason":"runtime.stop_failed","failedAtMs":stopped_at
                    }),
                    predecessor_event_id: Some(stop_started.event_id.clone()),
                    source_event_ids: vec![stop_started.event_id.clone()],
                    created_at_ms: stopped_at,
                })?,
                true,
            )
        }
    };

    let cleanup_at = now_ms();
    let cleanup = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseCleanupStarted,
        payload: json!({"releaseId":release_id,"startedAtMs":cleanup_at}),
        predecessor_event_id: Some(stopped.event_id.clone()),
        source_event_ids: vec![stopped.event_id.clone()],
        created_at_ms: cleanup_at,
    })?;
    let session_claim_released = input
        .session_claim_event
        .map(|grant| release_claim(journal, grant, &release_id, &cleanup, true))
        .transpose()?;
    let request_claim_released = input
        .request_claim_event
        .map(|grant| {
            release_request_claim(
                journal,
                grant,
                &release_id,
                &cleanup,
                session_claim_released.as_ref(),
            )
        })
        .transpose()?;
    let claim_source = request_claim_released
        .as_ref()
        .or(session_claim_released.as_ref())
        .unwrap_or(&cleanup);
    let lease_released = release_lease(journal, input.lease_event, &release_id, claim_source)?;
    let owner_released = release_owner(
        journal,
        input.owner_event,
        &release_id,
        &lease_released,
        &stopped,
    )?;

    let committed_at = now_ms();
    let committed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Release,
        aggregate_id: release_id.clone(),
        event_type: EventType::ReleaseCleanupCommitted,
        payload: json!({
            "releaseId":release_id,
            "requestClaimReleaseMode":if request_claim_released.is_some(){"released"}else{"not_applicable"},
            "sessionClaimReleaseMode":if session_claim_released.is_some(){"released"}else{"not_applicable"},
            "leaseReleaseMode":"released",
            "ownerReleaseMode":"released","committedAtMs":committed_at
        }),
        predecessor_event_id: Some(cleanup.event_id.clone()),
        source_event_ids: release_commit_sources(
            request_claim_released.as_ref(),
            session_claim_released.as_ref(),
            &lease_released,
            &owner_released,
        ),
        created_at_ms: committed_at,
    })?;
    let standby_at = now_ms();
    let active_cooldown = if stop_failed {
        None
    } else {
        journal
            .replay()?
            .state
            .slots
            .get(input.slot_id)
            .and_then(|slot| slot.cooldown_until_ms)
            .filter(|until| *until > standby_at)
    };
    let allocatable = !stop_failed && active_cooldown.is_none();
    let standby = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: input.slot_id.to_string(),
        event_type: EventType::SlotStandbyWritten,
        payload: json!({
            "slotId":input.slot_id,"releaseId":release_id,"allocatable":allocatable,
            "cooldownUntilMs":active_cooldown,"writtenAtMs":standby_at
        }),
        predecessor_event_id: Some(input.slot_predecessor.event_id.clone()),
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
                "releaseId":release_id,
                "finalStatus":if stop_failed{"cleanup_failed"}else{"allocatable"},
                "allocatable":allocatable,
                "finalizedAtMs":finalized_at
            }),
            predecessor_event_id: Some(committed.event_id),
            source_event_ids: vec![standby.event_id],
            created_at_ms: finalized_at,
        })?;
    }
    Ok(ReleaseCompletion { stop_failed })
}

fn release_claim(
    journal: &mut SessionJournal,
    grant: &EventEnvelope,
    release_id: &str,
    cleanup: &EventEnvelope,
    session_operation: bool,
) -> Result<EventEnvelope, SessionReleaseError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Claim,
        aggregate_id: grant.payload["claimId"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        event_type: if session_operation {
            EventType::SessionOperationClaimReleased
        } else {
            EventType::RequestClaimReleased
        },
        payload: json!({
            "claimId":grant.payload["claimId"],"claimGeneration":1,
            "releaseId":release_id,"releasedAtMs":at
        }),
        predecessor_event_id: Some(grant.event_id.clone()),
        source_event_ids: vec![cleanup.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn release_request_claim(
    journal: &mut SessionJournal,
    grant: &EventEnvelope,
    release_id: &str,
    cleanup: &EventEnvelope,
    session_claim_release: Option<&EventEnvelope>,
) -> Result<EventEnvelope, SessionReleaseError> {
    if let Some(session_release) = session_claim_release {
        let at = now_ms();
        return Ok(journal.append(NewEvent {
            aggregate_kind: AggregateKind::Claim,
            aggregate_id: grant.payload["claimId"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            event_type: EventType::RequestClaimReleased,
            payload: json!({
                "claimId":grant.payload["claimId"],"claimGeneration":1,
                "releaseId":release_id,"releasedAtMs":at
            }),
            predecessor_event_id: Some(grant.event_id.clone()),
            source_event_ids: vec![cleanup.event_id.clone(), session_release.event_id.clone()],
            created_at_ms: at,
        })?);
    }
    release_claim(journal, grant, release_id, cleanup, false)
}

fn release_commit_sources(
    request_claim: Option<&EventEnvelope>,
    session_claim: Option<&EventEnvelope>,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
) -> Vec<String> {
    let mut sources = Vec::with_capacity(4);
    if let Some(event) = request_claim {
        sources.push(event.event_id.clone());
    }
    if let Some(event) = session_claim {
        sources.push(event.event_id.clone());
    }
    sources.push(lease.event_id.clone());
    sources.push(owner.event_id.clone());
    sources
}

fn release_lease(
    journal: &mut SessionJournal,
    grant: &EventEnvelope,
    release_id: &str,
    claim_release: &EventEnvelope,
) -> Result<EventEnvelope, SessionReleaseError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Lease,
        aggregate_id: grant.payload["leaseId"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        event_type: EventType::SlotLeaseReleased,
        payload: json!({
            "leaseId":grant.payload["leaseId"],"leaseGeneration":1,
            "releaseId":release_id,"releasedAtMs":at
        }),
        predecessor_event_id: Some(grant.event_id.clone()),
        source_event_ids: vec![claim_release.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn release_owner(
    journal: &mut SessionJournal,
    grant: &EventEnvelope,
    release_id: &str,
    lease_release: &EventEnvelope,
    stopped: &EventEnvelope,
) -> Result<EventEnvelope, SessionReleaseError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::RuntimeOwner,
        aggregate_id: grant.payload["runtimeOwnerId"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        event_type: EventType::RuntimeOwnershipReleased,
        payload: json!({
            "runtimeOwnerId":grant.payload["runtimeOwnerId"],"ownerGeneration":1,
            "releaseId":release_id,
            "runtimeOutcome":runtime_outcome_literal(stopped.event_type),"releasedAtMs":at
        }),
        predecessor_event_id: Some(grant.event_id.clone()),
        source_event_ids: vec![lease_release.event_id.clone(), stopped.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn release_evidence_root(
    config: &SupervisorConfig,
    request_key: &str,
    operation_id: &str,
) -> Result<PathBuf, std::io::Error> {
    let root = config
        .state_root
        .join("evidence/requests")
        .join(request_key)
        .join("operations")
        .join(format!("{operation_id}.release"));
    crate::provider_runner::create_private_directory(&config.state_root, &root)?;
    Ok(root)
}

fn write_evidence_manifest(
    state_root: &Path,
    evidence_root: &Path,
    event_ids: &[String],
    receipt_ids: &[String],
) -> Result<(String, String), SessionReleaseError> {
    let bytes = canonical_bytes(&json!({"eventIds":event_ids,"receiptIds":receipt_ids}))?;
    let target = evidence_root.join("evidence-manifest.json");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            File::open(evidence_root)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(&target)? != bytes {
                return Err(error.into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    let relative = target
        .strip_prefix(state_root)
        .map_err(std::io::Error::other)?
        .to_str()
        .ok_or_else(|| std::io::Error::other("non-UTF8 evidence path"))?
        .replace('\\', "/");
    Ok((relative, h256(bytes)))
}
