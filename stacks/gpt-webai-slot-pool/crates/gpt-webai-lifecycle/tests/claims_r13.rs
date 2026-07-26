use gpt_webai_lifecycle::claims::renewal::{release, renew, verify_active};
use gpt_webai_lifecycle::claims::request::grant_request_claim;
use gpt_webai_lifecycle::claims::session_operation::{
    grant_session_operation_claim, SessionOperationClaimInput,
};
use gpt_webai_lifecycle::claims::{fencing_hash, CasError, RENEW_CADENCE_MS, RESOURCE_TTL_MS};
use gpt_webai_lifecycle::contracts::events::Writer;

#[test]
fn request_claim_uses_the_closed_ttl_and_renewal_cadence() {
    let claim = grant_request_claim(
        std::iter::empty(),
        "request-1",
        "operation-1",
        "fence-1",
        writer(),
        1,
        event_id('1'),
    )
    .expect("grant request claim");

    assert!(claim.id.starts_with("claim_"));
    assert_eq!(claim.generation, 1);
    assert_eq!(claim.renewal_revision, 1);
    assert_eq!(claim.fencing_token_sha256, Some(fencing_hash("fence-1")));
    assert_eq!(claim.renew_at_ms, 1 + RENEW_CADENCE_MS);
    assert_eq!(claim.expires_at_ms, 1 + RESOURCE_TTL_MS);

    let renewed = renew(
        &claim,
        claim.generation,
        Some("fence-1"),
        claim.renew_at_ms,
        event_id('2'),
    )
    .expect("renew claim");
    assert_eq!(renewed.generation, claim.generation);
    assert_eq!(renewed.renewal_revision, 2);
    assert_eq!(renewed.owner, claim.owner);
    assert_eq!(renewed.renew_at_ms, claim.renew_at_ms + RENEW_CADENCE_MS);
    assert_eq!(renewed.expires_at_ms, claim.renew_at_ms + RESOURCE_TTL_MS);
}

#[test]
fn claim_renewal_fails_closed_at_expiry_on_bad_token_and_on_clock_reversal() {
    let claim = request_claim();

    assert_eq!(
        renew(
            &claim,
            claim.generation,
            Some("fence-1"),
            claim.expires_at_ms - 1,
            event_id('2')
        )
        .expect("renew before expiry")
        .renewal_revision,
        2
    );
    assert_eq!(
        renew(
            &claim,
            claim.generation,
            Some("fence-1"),
            claim.expires_at_ms,
            event_id('2')
        ),
        Err(CasError::Expired)
    );
    assert_eq!(
        renew(
            &claim,
            claim.generation,
            Some("wrong"),
            claim.renew_at_ms,
            event_id('2')
        ),
        Err(CasError::FencingMismatch)
    );
    assert_eq!(
        renew(
            &claim,
            claim.generation,
            Some("fence-1"),
            claim.granted_at_ms - 1,
            event_id('2')
        ),
        Err(CasError::Invalid("clock reversal"))
    );
    assert_eq!(
        renew(
            &claim,
            claim.generation,
            Some("fence-1"),
            claim.renew_at_ms,
            "not-an-event".to_string()
        ),
        Err(CasError::Invalid("renewal eventId"))
    );
}

#[test]
fn release_is_exactly_once_for_the_active_generation() {
    let claim = request_claim();
    let released = release(&claim, claim.generation, Some("fence-1"), 2, event_id('2'))
        .expect("release claim");

    assert_eq!(released.status, "released");
    assert_eq!(released.released_at_ms, Some(2));
    assert_eq!(released.release_event_id, Some(event_id('2')));
    assert_eq!(
        release(
            &released,
            released.generation,
            Some("fence-1"),
            3,
            event_id('3')
        ),
        Err(CasError::Inactive)
    );
    assert_eq!(
        verify_active(&claim, claim.generation + 1, Some("fence-1"), 2),
        Err(CasError::GenerationMismatch)
    );
}

#[test]
fn request_and_session_subject_conflicts_are_rejected_without_mutation() {
    let request = request_claim();
    assert!(matches!(
        grant_request_claim(
            [&request],
            "request-1",
            "operation-2",
            "fence-1",
            writer(),
            2,
            event_id('2')
        ),
        Err(CasError::SubjectConflict(subject)) if subject == "request-1"
    ));

    let session = grant_session_operation_claim(
        std::iter::empty(),
        SessionOperationClaimInput {
            session_id: "session_1",
            operation_id: "operation-1",
            operation_kind: "resume",
            fencing_token: "fence-1",
            owner: writer(),
            now_ms: 1,
            event_id: event_id('1'),
        },
    )
    .expect("grant session claim");
    assert!(matches!(
        grant_session_operation_claim(
            [&session],
            SessionOperationClaimInput {
                session_id: "session_1",
                operation_id: "operation-2",
                operation_kind: "show",
                fencing_token: "fence-1",
                owner: writer(),
                now_ms: 2,
                event_id: event_id('2'),
            }
        ),
        Err(CasError::SubjectConflict(subject)) if subject == "session_1"
    ));
}

#[test]
fn malformed_claim_inputs_are_rejected_before_id_derivation() {
    assert_eq!(
        grant_request_claim(
            std::iter::empty(),
            "",
            "operation-1",
            "fence-1",
            writer(),
            1,
            event_id('1')
        ),
        Err(CasError::Invalid("request claim input"))
    );
    assert_eq!(
        grant_session_operation_claim(
            std::iter::empty(),
            SessionOperationClaimInput {
                session_id: "session_1",
                operation_id: "operation-1",
                operation_kind: "allocate",
                fencing_token: "fence-1",
                owner: writer(),
                now_ms: 1,
                event_id: event_id('1'),
            }
        ),
        Err(CasError::Invalid("operationKind"))
    );
}

fn request_claim() -> gpt_webai_lifecycle::contracts::projection::CasRecord {
    grant_request_claim(
        std::iter::empty(),
        "request-1",
        "operation-1",
        "fence-1",
        writer(),
        1,
        event_id('1'),
    )
    .expect("request claim")
}

fn writer() -> Writer {
    Writer {
        host_id: format!("host_{}", "1".repeat(32)),
        process_id: 1,
        process_start_ms: 1,
        writer_id: format!("writer_{}", "2".repeat(64)),
    }
}

fn event_id(value: char) -> String {
    format!("evt_{}", value.to_string().repeat(64))
}
