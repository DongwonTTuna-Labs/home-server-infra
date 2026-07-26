use gpt_webai_lifecycle::claims::renewal::renew;
use gpt_webai_lifecycle::claims::session_operation::{
    grant_session_operation_claim, SessionOperationClaimInput,
};
use gpt_webai_lifecycle::claims::{fencing_hash, grant, CasError, CasKind, GrantInput};
use gpt_webai_lifecycle::contracts::events::Writer;
use gpt_webai_lifecycle::runtime::ownership::{grant_ownership, OwnershipGrant};

const SESSION_ID: &str = "6a623c19-bb00-83ee-bb64-691d8bff937b";

#[test]
fn persisted_session_operation_claim_is_exclusive_and_fenced() {
    let claim = session_claim(&[], "operation-1", "show", event_id('1'));
    assert_eq!(claim.kind, "claim");
    assert_eq!(claim.subject_id, SESSION_ID);
    assert_eq!(claim.generation, 1);
    assert_eq!(claim.fencing_token_sha256, Some(fencing_hash("fence-1")));
    assert_eq!(claim.renew_at_ms, 101_000);
    assert_eq!(claim.expires_at_ms, 301_000);

    let conflict = grant_session_operation_claim(
        [&claim],
        SessionOperationClaimInput {
            session_id: SESSION_ID,
            operation_id: "operation-2",
            operation_kind: "resume",
            fencing_token: "fence-2",
            owner: writer(),
            now_ms: 2_000,
            event_id: event_id('2'),
        },
    )
    .expect_err("only one active session operation claim is legal");
    assert_eq!(conflict, CasError::SubjectConflict(SESSION_ID.to_string()));
}

#[test]
fn persisted_session_resources_remain_pinned_to_one_slot() {
    let claim = session_claim(&[], "operation-1", "download", event_id('1'));
    let lease = grant(GrantInput {
        id: prefixed("lease", '2'),
        kind: CasKind::SlotLease,
        subject_id: "slot-04".to_string(),
        owner: writer(),
        generation: 3,
        fencing_token_sha256: Some(fencing_hash("fence-1")),
        now_ms: 1_000,
        event_id: event_id('2'),
    })
    .unwrap();
    let owner = grant_ownership(OwnershipGrant {
        slot_id: "slot-04",
        operation_id: "operation-1",
        runtime_incarnation_id: &prefixed("runtime", '3'),
        docker_status: "running",
        fencing_token: "fence-1",
        owner: writer(),
        generation: 4,
        now_ms: 1_000,
        event_id: event_id('3'),
    })
    .unwrap();

    assert_eq!(claim.subject_id, SESSION_ID);
    assert_eq!(lease.subject_id, "slot-04");
    assert_eq!(owner.cas.subject_id, lease.subject_id);
    assert_eq!(owner.runtime_incarnation_id, prefixed("runtime", '3'));
}

#[test]
fn claim_lease_and_owner_renew_on_the_same_hundred_second_cadence() {
    let claim = session_claim(&[], "operation-1", "poll", event_id('1'));
    let lease = grant(GrantInput {
        id: prefixed("lease", '2'),
        kind: CasKind::SlotLease,
        subject_id: "slot-01".to_string(),
        owner: writer(),
        generation: 1,
        fencing_token_sha256: Some(fencing_hash("fence-1")),
        now_ms: 1_000,
        event_id: event_id('2'),
    })
    .unwrap();
    let owner = grant(GrantInput {
        id: prefixed("owner", '3'),
        kind: CasKind::RuntimeOwner,
        subject_id: "slot-01".to_string(),
        owner: writer(),
        generation: 1,
        fencing_token_sha256: Some(fencing_hash("fence-1")),
        now_ms: 1_000,
        event_id: event_id('3'),
    })
    .unwrap();

    for (record, event) in [(&claim, '4'), (&lease, '5'), (&owner, '6')] {
        let renewed = renew(record, 1, Some("fence-1"), 101_000, event_id(event)).unwrap();
        assert_eq!(renewed.id, record.id);
        assert_eq!(renewed.renewal_revision, 2);
        assert_eq!(renewed.renew_at_ms, 201_000);
        assert_eq!(renewed.expires_at_ms, 401_000);
    }
}

#[test]
fn renewal_failure_is_fail_closed_for_every_persisted_resource() {
    let claim = session_claim(&[], "operation-1", "resume", event_id('1'));
    assert_eq!(
        renew(&claim, 1, Some("wrong"), 101_000, event_id('2')).unwrap_err(),
        CasError::FencingMismatch
    );
    assert_eq!(
        renew(&claim, 2, Some("fence-1"), 101_000, event_id('3')).unwrap_err(),
        CasError::GenerationMismatch
    );
    assert_eq!(
        renew(
            &claim,
            1,
            Some("fence-1"),
            claim.expires_at_ms,
            event_id('4')
        )
        .unwrap_err(),
        CasError::Expired
    );
}

fn session_claim<'a>(
    existing: impl IntoIterator<Item = &'a gpt_webai_lifecycle::contracts::projection::CasRecord>,
    operation_id: &str,
    operation_kind: &str,
    event_id: String,
) -> gpt_webai_lifecycle::contracts::projection::CasRecord {
    grant_session_operation_claim(
        existing,
        SessionOperationClaimInput {
            session_id: SESSION_ID,
            operation_id,
            operation_kind,
            fencing_token: "fence-1",
            owner: writer(),
            now_ms: 1_000,
            event_id,
        },
    )
    .unwrap()
}

fn writer() -> Writer {
    Writer {
        host_id: format!("host_{}", "1".repeat(32)),
        process_id: 1,
        process_start_ms: 1,
        writer_id: prefixed("writer", '1'),
    }
}

fn prefixed(prefix: &str, value: char) -> String {
    format!("{prefix}_{}", value.to_string().repeat(64))
}

fn event_id(value: char) -> String {
    prefixed("evt", value)
}
