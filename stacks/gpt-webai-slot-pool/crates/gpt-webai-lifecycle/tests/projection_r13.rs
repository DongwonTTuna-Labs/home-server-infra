#[path = "journal_r13/fixtures.rs"]
mod fixtures;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use fixtures::{event, h, request_accepted, request_claim, TempRoot};
use gpt_webai_lifecycle::contracts::events::{AggregateKind, EventType};
use gpt_webai_lifecycle::journal::head::{Head, HEAD_SCHEMA};
use gpt_webai_lifecycle::journal::projection::{empty_files, projection_digest, reduce};
use gpt_webai_lifecycle::journal::{HeadStore, Snapshot, SnapshotStore};
use serde_json::json;

#[test]
fn ten_file_digest_is_ordered_and_replay_stable() {
    let empty = empty_files();
    let first = projection_digest(&empty).expect("empty digest");
    let second = projection_digest(&empty).expect("repeat digest");
    assert_eq!(first, second);
    assert!(first.starts_with("sha256:"));
    assert_eq!(empty.keys().count(), 10);

    let accepted = request_accepted("request-projection", 1_000);
    let a = reduce(std::slice::from_ref(&accepted), &BTreeMap::new()).expect("replay");
    let b = reduce(&[accepted], &BTreeMap::new()).expect("repeat replay");
    assert_eq!(a, b);
    assert_eq!(a.state.last_event_created_at_ms, 1_000);
}

#[test]
fn cas_renewal_changes_only_revision_and_deadlines() {
    let accepted = request_accepted("request-renew", 10_000);
    let claim = request_claim("request-renew", &accepted, '8', 11_000);
    let claim_id = claim.payload["claimId"]
        .as_str()
        .expect("claim id")
        .to_string();
    let renewed = event(
        AggregateKind::Claim,
        &claim_id,
        EventType::RequestClaimRenewed,
        111_000,
        json!({
            "claimId":claim_id,"claimGeneration":1,"renewalRevision":2,
            "renewedAtMs":111000,"renewAtMs":211000,"expiresAtMs":411000
        }),
        Some(claim.event_id.clone()),
        Some("request-renew"),
        vec![claim.event_id.clone()],
    );
    let reduced = reduce(&[accepted, claim, renewed], &BTreeMap::new()).expect("renew replay");
    let record = &reduced.state.claims[&claim_id];
    assert_eq!(record.generation, 1);
    assert_eq!(record.renewal_revision, 2);
    assert_eq!(record.granted_at_ms, 11_000);
    assert_eq!(record.expires_at_ms, 411_000);
    assert_eq!(record.status, "active");
}

#[test]
fn qa_reducer_enforces_next_index_and_case_reset() {
    let matrix = qa_event(
        EventType::QaMatrixRecorded,
        20_000,
        json!({
            "qaRunId":"qa-run-1","matrixIteration":1,"sourceFingerprint":h('e'),
            "evidenceDigest":h('f'),"casesPassed":21,"casesTotal":21,"recordedAtMs":20000
        }),
        None,
    );
    let repeat = qa_event(
        EventType::QaRepeatRecorded,
        21_000,
        json!({
            "qaRunId":"qa-run-2","caseId":"R01","repetitionIndex":1,
            "sourceFingerprint":h('e'),"passed":true,"recordedAtMs":21000
        }),
        Some(matrix.event_id.clone()),
    );
    let reset = qa_event(
        EventType::QaCountersReset,
        22_000,
        json!({
            "qaRunId":"qa-run-3","reason":"case failed","sourceFingerprint":h('e'),
            "scope":"case","caseId":"R01","resetAtMs":22000
        }),
        Some(repeat.event_id.clone()),
    );
    let reduced = reduce(&[matrix, repeat, reset], &BTreeMap::new()).expect("QA replay");
    let counter = &reduced.state.qa_counters["qa"];
    assert_eq!(counter.matrix_iterations_passed, 1);
    assert_eq!(counter.repeat_counts["R01"], 0);
    assert!(counter.last_reset_event_id.is_some());
}

#[test]
fn qa_reducer_rejects_nonconsecutive_repeat() {
    let repeat = qa_event(
        EventType::QaRepeatRecorded,
        30_000,
        json!({
            "qaRunId":"qa-run-gap","caseId":"R01","repetitionIndex":2,
            "sourceFingerprint":h('1'),"passed":true,"recordedAtMs":30000
        }),
        None,
    );
    let error = reduce(&[repeat], &BTreeMap::new()).expect_err("gap rejected");
    assert!(error.to_string().contains("sequence mismatch"));
}

#[test]
fn snapshot_chain_requires_hash_and_replay_identity() {
    let root = TempRoot::new("snapshots");
    let accepted = request_accepted("request-snapshot", 40_000);
    let claim = request_claim("request-snapshot", &accepted, '9', 41_000);
    let first_projection = reduce(std::slice::from_ref(&accepted), &BTreeMap::new())
        .expect("first projection")
        .state;
    let first = Snapshot::new(first_projection, None, 42_000).expect("first snapshot");
    let head_store = HeadStore::new(root.path());
    let guard = head_store.acquire_mutation().expect("snapshot lock");
    let snapshots = SnapshotStore::new(root.path());
    let (_, first_sha) = snapshots
        .publish(&guard, "op-snapshot-1", &first)
        .expect("publish first");
    let second_projection = reduce(&[accepted.clone(), claim.clone()], &BTreeMap::new())
        .expect("second projection")
        .state;
    let second =
        Snapshot::new(second_projection, Some(first_sha), 43_000).expect("second snapshot");
    let (_, second_sha) = snapshots
        .publish(&guard, "op-snapshot-2", &second)
        .expect("publish second");
    let head = Head {
        head_generation: 1,
        last_event_id: Some(claim.event_id.clone()),
        projection_digest: second.projection_digest.clone(),
        schema_version: HEAD_SCHEMA.to_string(),
        snapshot_event_id: Some(second.last_event_id.clone()),
        snapshot_sha256: Some(second_sha),
        updated_at_ms: 44_000,
    };
    assert_eq!(
        snapshots
            .load_trusted(&head, &[accepted, claim], &BTreeMap::new())
            .expect("trusted")
            .last_event_id,
        second.last_event_id
    );
}

#[test]
fn corrupt_snapshot_is_ignored_for_full_replay_fallback() {
    let root = TempRoot::new("snapshot-corrupt");
    let accepted = request_accepted("request-corrupt", 50_000);
    let projection = reduce(std::slice::from_ref(&accepted), &BTreeMap::new())
        .expect("projection")
        .state;
    let snapshot = Snapshot::new(projection, None, 51_000).expect("snapshot");
    let head_store = HeadStore::new(root.path());
    let guard = head_store.acquire_mutation().expect("lock");
    let snapshots = SnapshotStore::new(root.path());
    let (path, sha) = snapshots
        .publish(&guard, "op-corrupt", &snapshot)
        .expect("publish");
    fs::write(&path, b"{}\n").expect("corrupt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
    let head = Head {
        head_generation: 1,
        last_event_id: Some(accepted.event_id.clone()),
        projection_digest: snapshot.projection_digest.clone(),
        schema_version: HEAD_SCHEMA.to_string(),
        snapshot_event_id: Some(snapshot.last_event_id),
        snapshot_sha256: Some(sha),
        updated_at_ms: 52_000,
    };
    assert!(snapshots
        .load_trusted(&head, &[accepted], &BTreeMap::new())
        .is_none());
}

#[test]
fn snapshot_chain_rejects_a_previous_snapshot_from_a_later_event() {
    let root = TempRoot::new("snapshot-reversed-event-order");
    let accepted = request_accepted("request-snapshot-reversed", 60_000);
    let claim = request_claim("request-snapshot-reversed", &accepted, 'a', 61_000);
    let later_projection = reduce(&[accepted.clone(), claim.clone()], &BTreeMap::new())
        .expect("later projection")
        .state;
    let later = Snapshot::new(later_projection, None, 62_000).expect("later snapshot");
    let head_store = HeadStore::new(root.path());
    let guard = head_store.acquire_mutation().expect("snapshot lock");
    let snapshots = SnapshotStore::new(root.path());
    let (_, later_sha) = snapshots
        .publish(&guard, "op-snapshot-later", &later)
        .expect("publish later");

    let earlier_projection = reduce(std::slice::from_ref(&accepted), &BTreeMap::new())
        .expect("earlier projection")
        .state;
    let reversed =
        Snapshot::new(earlier_projection, Some(later_sha), 63_000).expect("reversed snapshot");
    let (_, reversed_sha) = snapshots
        .publish(&guard, "op-snapshot-reversed", &reversed)
        .expect("publish reversed");
    let head = Head {
        head_generation: 1,
        last_event_id: Some(claim.event_id.clone()),
        projection_digest: reversed.projection_digest.clone(),
        schema_version: HEAD_SCHEMA.to_string(),
        snapshot_event_id: Some(reversed.last_event_id.clone()),
        snapshot_sha256: Some(reversed_sha),
        updated_at_ms: 64_000,
    };
    assert!(snapshots
        .load_trusted(&head, &[accepted, claim], &BTreeMap::new())
        .is_none());
}

fn qa_event(
    kind: EventType,
    at_ms: u64,
    payload: serde_json::Value,
    predecessor: Option<String>,
) -> gpt_webai_lifecycle::contracts::events::EventEnvelope {
    event(
        AggregateKind::Qa,
        "qa",
        kind,
        at_ms,
        payload,
        predecessor,
        None,
        vec![],
    )
}
