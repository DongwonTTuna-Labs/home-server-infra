use gpt_webai_lifecycle::claims::{fencing_hash, grant, CasKind, GrantInput};
use gpt_webai_lifecycle::contracts::events::Writer;
use gpt_webai_lifecycle::contracts::ids::h256;
use gpt_webai_lifecycle::contracts::projection::RuntimeOwnerRecord;
use gpt_webai_lifecycle::release::cleanup::ReleaseMachine;
use gpt_webai_lifecycle::release::ownership::{
    authorize_stop, StopAuthorization, StopAuthorizationInput,
};
use gpt_webai_lifecycle::release::{
    EvidenceManifest, ReleaseError, ReleaseFinalStatus, ReleaseReason, ReleaseStart,
    ReleaseSubjectKind, ResourceKind, RuntimeOutcome,
};
use gpt_webai_lifecycle::runtime::ownership::DeadOwnerProof;

#[test]
fn evidence_precedes_stop_cleanup_and_exactly_once_release() {
    let mut release = machine([
        ResourceKind::RequestClaim,
        ResourceKind::SlotLease,
        ResourceKind::RuntimeOwner,
    ]);
    assert!(release
        .record_runtime_outcome(RuntimeOutcome::Stopped)
        .is_err());
    release.preserve_evidence(evidence()).unwrap();
    release
        .record_runtime_outcome(RuntimeOutcome::Stopped)
        .unwrap();
    release.start_cleanup().unwrap();
    release
        .release_resource(ResourceKind::RuntimeOwner)
        .unwrap();
    assert!(release
        .release_resource(ResourceKind::RuntimeOwner)
        .is_err());
    release.release_resource(ResourceKind::SlotLease).unwrap();
    release
        .release_resource(ResourceKind::RequestClaim)
        .unwrap();
    release.commit_cleanup().unwrap();
    release.write_standby(true).unwrap();
    release
        .finalize(ReleaseFinalStatus::Allocatable, true, 2_000)
        .unwrap();
    assert!(release.all_resources_released());
    assert_eq!(
        release.final_status(),
        Some(ReleaseFinalStatus::Allocatable)
    );
}

#[test]
fn partial_claim_only_release_has_no_standby_and_no_slot_status() {
    let mut release = machine([ResourceKind::SessionClaim]);
    release.preserve_evidence(evidence()).unwrap();
    release
        .record_runtime_outcome(RuntimeOutcome::SkippedNotAcquired)
        .unwrap();
    release.start_cleanup().unwrap();
    release
        .release_resource(ResourceKind::SessionClaim)
        .unwrap();
    release.commit_cleanup().unwrap();
    assert!(release.write_standby(false).is_err());
    release
        .finalize(ReleaseFinalStatus::ResourcesReleasedNoSlot, false, 2_000)
        .unwrap();
}

#[test]
fn stop_failure_forces_cleanup_failed_nonallocatable() {
    let mut release = machine([ResourceKind::SlotLease, ResourceKind::RuntimeOwner]);
    release.preserve_evidence(evidence()).unwrap();
    release
        .record_runtime_outcome(RuntimeOutcome::Failed)
        .unwrap();
    release.start_cleanup().unwrap();
    release
        .release_resource(ResourceKind::RuntimeOwner)
        .unwrap();
    release.release_resource(ResourceKind::SlotLease).unwrap();
    release.commit_cleanup().unwrap();
    release.write_standby(false).unwrap();
    assert!(release
        .finalize(ReleaseFinalStatus::Allocatable, true, 2_000)
        .is_err());
    release
        .finalize(ReleaseFinalStatus::CleanupFailed, false, 2_000)
        .unwrap();
}

#[test]
fn cooldown_is_the_only_replaceable_final_status() {
    let mut release = machine([ResourceKind::SlotLease]);
    release.preserve_evidence(evidence()).unwrap();
    release
        .record_runtime_outcome(RuntimeOutcome::SkippedNotAcquired)
        .unwrap();
    release.start_cleanup().unwrap();
    release.release_resource(ResourceKind::SlotLease).unwrap();
    release.commit_cleanup().unwrap();
    release.write_standby(false).unwrap();
    release
        .finalize(ReleaseFinalStatus::CooldownBlocked, false, 2_000)
        .unwrap();
    release
        .clear_cooldown_and_finalize_allocatable(3_000)
        .unwrap();
    assert_eq!(
        release.final_status(),
        Some(ReleaseFinalStatus::Allocatable)
    );
}

#[test]
fn current_fenced_owner_can_stop_but_mismatch_cannot() {
    let owner = runtime_owner(Some(fencing_hash("token")), 1_000);
    let authorized = authorize_stop(StopAuthorizationInput {
        owner: Some(&owner),
        presented_generation: Some(1),
        fencing_token: Some("token"),
        now_ms: 2_000,
        dead_owner_proof: None,
        release_id: &release_id(),
        takeover_writer: writer(),
        takeover_event_id: event_id('9'),
    })
    .unwrap();
    assert!(matches!(authorized, StopAuthorization::CurrentOwner(_)));

    let denied = authorize_stop(StopAuthorizationInput {
        owner: Some(&owner),
        presented_generation: Some(1),
        fencing_token: Some("wrong"),
        now_ms: 2_000,
        dead_owner_proof: None,
        release_id: &release_id(),
        takeover_writer: writer(),
        takeover_event_id: event_id('8'),
    })
    .expect_err("wrong token is a fencing failure");
    assert_eq!(denied, ReleaseError::FencingMismatch);
}

#[test]
fn wrong_fencing_token_cannot_be_upgraded_to_tokenless_takeover() {
    let owner = runtime_owner(Some(fencing_hash("token")), 1_000);
    let proof = dead_owner_proof(&owner);
    let error = authorize_stop(StopAuthorizationInput {
        owner: Some(&owner),
        presented_generation: Some(1),
        fencing_token: Some("wrong"),
        now_ms: proof.proven_at_ms,
        dead_owner_proof: Some(&proof),
        release_id: &release_id(),
        takeover_writer: writer(),
        takeover_event_id: event_id('6'),
    })
    .expect_err("a presented invalid fence is not tokenless");
    assert_eq!(error, ReleaseError::FencingMismatch);
}

#[test]
fn expired_dead_owner_requires_takeover_before_stop() {
    let owner = runtime_owner(None, 1_000);
    let proof = dead_owner_proof(&owner);
    let result = authorize_stop(StopAuthorizationInput {
        owner: Some(&owner),
        presented_generation: None,
        fencing_token: None,
        now_ms: proof.proven_at_ms,
        dead_owner_proof: Some(&proof),
        release_id: &release_id(),
        takeover_writer: writer(),
        takeover_event_id: event_id('7'),
    })
    .unwrap();
    match result {
        StopAuthorization::Takeover {
            retired,
            replacement,
        } => {
            assert_eq!(retired.cas.status, "released");
            assert_eq!(replacement.cas.generation, 2);
            assert!(replacement.cas.fencing_token_sha256.is_none());
        }
        _ => panic!("takeover required"),
    }
}

fn dead_owner_proof(owner: &RuntimeOwnerRecord) -> DeadOwnerProof {
    DeadOwnerProof {
        prior_owner_id: owner.cas.id.clone(),
        prior_generation: 1,
        expired_at_ms: owner.cas.expires_at_ms,
        grace_satisfied_at_ms: owner.cas.expires_at_ms + 30_000,
        process_absent: true,
        container_label_owner_id: None,
        container_label_generation: None,
        lease_inactive: true,
        claim_inactive: true,
        evidence_refs: vec![gpt_webai_lifecycle::contracts::browser::EvidenceRef {
            path: "requests/r/operations/release/dead-owner.json".to_string(),
            sha256: h256(b"proof"),
            size_bytes: 1,
            media_type: gpt_webai_lifecycle::contracts::browser::EvidenceMediaType::Json,
        }],
        proven_at_ms: owner.cas.expires_at_ms + 30_001,
    }
}

fn machine(acquired: impl IntoIterator<Item = ResourceKind>) -> ReleaseMachine {
    ReleaseMachine::start(
        ReleaseStart {
            release_id: release_id(),
            subject_kind: ReleaseSubjectKind::Request,
            subject_id: "request-1".to_string(),
            reason: ReleaseReason::OutputPublished,
            started_at_ms: 1_000,
        },
        acquired,
    )
    .unwrap()
}

fn evidence() -> EvidenceManifest {
    EvidenceManifest {
        path: "requests/r/evidence-manifest.json".to_string(),
        sha256: h256(b"manifest"),
        preserved_at_ms: 1_100,
    }
}

fn runtime_owner(token: Option<String>, now_ms: u64) -> RuntimeOwnerRecord {
    let cas = grant(GrantInput {
        id: format!("owner_{}", "1".repeat(64)),
        kind: CasKind::RuntimeOwner,
        subject_id: "slot-01".to_string(),
        owner: writer(),
        generation: 1,
        fencing_token_sha256: token,
        now_ms,
        event_id: event_id('1'),
    })
    .unwrap();
    RuntimeOwnerRecord {
        cas,
        runtime_incarnation_id: format!("runtime_{}", "2".repeat(64)),
        docker_status: "running".to_string(),
    }
}

fn writer() -> Writer {
    Writer {
        host_id: format!("host_{}", "1".repeat(32)),
        process_id: 1,
        process_start_ms: 1,
        writer_id: format!("writer_{}", "2".repeat(64)),
    }
}

fn release_id() -> String {
    format!("release_{}", "3".repeat(64))
}

fn event_id(value: char) -> String {
    format!("evt_{}", value.to_string().repeat(64))
}
