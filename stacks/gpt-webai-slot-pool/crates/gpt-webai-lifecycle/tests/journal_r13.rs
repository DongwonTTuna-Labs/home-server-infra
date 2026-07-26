#[path = "journal_r13/fixtures.rs"]
mod fixtures;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use fixtures::{
    allocation, claim_renewed, event, evidence, explicit_release, h, lease, request_accepted,
    request_claim, runtime_owner, staged, TempRoot,
};
use gpt_webai_lifecycle::contracts::events::{AggregateKind, EventType};
use gpt_webai_lifecycle::journal::projection::reduce;
use gpt_webai_lifecycle::journal::{EventStore, HeadStore, PersistedSessionSeed};
use serde_json::json;

#[test]
fn transaction_publishes_event_head_and_all_ten_projections() {
    let root = TempRoot::new("commit");
    let accepted = request_accepted("request-a", 1_000);
    let claim = request_claim("request-a", &accepted, '3', 2_000);
    let staged = staged("request-a", &accepted, &claim, 3_000);
    let allocation = allocation("request-a", &staged, 4_000);
    let lease = lease("request-a", &claim, &allocation, 5_000);
    let head_store = HeadStore::new(root.path());
    let guard = head_store.acquire_mutation().expect("mutation lock");
    let result = EventStore::new(root.path())
        .append_transaction_with_seeds(
            &guard,
            &[accepted, claim, staged, allocation, lease],
            &BTreeMap::new(),
        )
        .expect("commit");
    assert_eq!(
        result.projection.state.requests["request-a"].state,
        "allocated"
    );
    assert_eq!(
        result.projection.state.allocator["allocator"].cohort_cursor,
        1
    );
    assert_eq!(result.event_paths.len(), 5);
    assert!(result.head.last_event_id.is_some());
    assert_eq!(
        result.head.projection_digest,
        result.projection.state.projection_digest
    );
    assert_eq!(
        fs::read_dir(root.path().join("journal/projections"))
            .expect("projection dir")
            .count(),
        10
    );
}

#[test]
fn replay_rejects_two_active_request_claims_for_one_subject() {
    let accepted = request_accepted("request-b", 10_000);
    let first = request_claim("request-b", &accepted, '5', 11_000);
    let second = request_claim("request-b", &accepted, '6', 12_000);
    let error = reduce(&[accepted, first, second], &BTreeMap::new()).expect_err("conflict");
    assert!(error.to_string().contains("subject conflict"));
}

#[test]
fn corrupt_head_and_projection_are_rebuilt_from_immutable_events() {
    let root = TempRoot::new("rebuild");
    let accepted = request_accepted("request-c", 20_000);
    let store = EventStore::new(root.path());
    let head_store = HeadStore::new(root.path());
    {
        let guard = head_store.acquire_mutation().expect("lock");
        store.append(&guard, &accepted).expect("initial commit");
    }
    fs::write(root.path().join("journal/HEAD.json"), b"not-json\n").expect("corrupt HEAD");
    fs::set_permissions(
        root.path().join("journal/HEAD.json"),
        fs::Permissions::from_mode(0o600),
    )
    .expect("chmod");
    fs::write(
        root.path().join("journal/projections/requests.json"),
        b"{}\n",
    )
    .expect("corrupt projection");
    let before = store.inspect_derived(&BTreeMap::new()).expect("inspect");
    assert!(!before.head_matches);
    assert!(!before.projections_match);
    {
        let guard = head_store.acquire_mutation().expect("rebuild lock");
        store
            .rebuild_derived(&guard, &BTreeMap::new(), "op-rebuild", 30_000)
            .expect("rebuild");
    }
    let after = store
        .inspect_derived(&BTreeMap::new())
        .expect("inspect rebuilt");
    assert!(after.head_matches);
    assert!(after.projections_match);
}

#[test]
fn persisted_session_bootstrap_replays_without_binding_event() {
    let claim_id = format!("claim_{}", "7".repeat(64));
    let claim = event(
        AggregateKind::Claim,
        &claim_id,
        EventType::SessionOperationClaimGranted,
        40_000,
        json!({
            "claimId":claim_id,"sessionId":"session-a","operationKind":"show",
            "expectedSlotId":"slot-04","expectedCohort":"cohort-b",
            "expectedRuntimeOwnerGeneration":1,"requestId":null,"runId":null,
            "ttlMs":300000,"grantedAtMs":40000,"renewAtMs":140000,
            "expiresAtMs":340000,"fencingTokenSha256":h('d')
        }),
        None,
        None,
        vec![],
    );
    let failed = event(
        AggregateKind::Session,
        "session-a",
        EventType::SessionOperationFailed,
        41_000,
        json!({
            "sessionId":"session-a","sessionOperationClaimId":claim.payload["claimId"],
            "operationKind":"show","stage":"lease","reason":"session.pinned_slot_unavailable",
            "providerReceipt":null,"failedAtMs":41000
        }),
        None,
        None,
        vec![claim.event_id.clone()],
    );
    let seeds = BTreeMap::from([(
        "session-a".to_string(),
        PersistedSessionSeed {
            session_id: "session-a".to_string(),
            session_binding_id: None,
            conversation_url: "https://chatgpt.com/c/session-a".to_string(),
            slot_id: "slot-04".to_string(),
            cohort: "cohort-b".to_string(),
            page_binding_generation: None,
        },
    )]);
    let reduced = reduce(&[claim, failed], &seeds).expect("bootstrap replay");
    let session = &reduced.state.sessions["session-a"];
    assert_eq!(session.page_binding_generation, 1);
    assert_eq!(session.last_operation_kind.as_deref(), Some("show"));
    assert!(session.session_binding_id.is_none());
}

#[test]
fn event_directory_rejects_non_event_json_names() {
    let root = TempRoot::new("strict-name");
    let directory = root.path().join("journal/events");
    fs::create_dir_all(&directory).expect("events dir");
    fs::write(directory.join("junk.json"), b"{}\n").expect("junk");
    let error = EventStore::new(root.path())
        .load_all()
        .expect_err("unsafe file");
    assert!(error.to_string().contains("unsafe journal file"));
}

#[test]
fn explicit_release_requires_current_active_resource_sources_in_canonical_order() {
    let accepted = request_accepted("request-a", 70_000);
    let claim = request_claim("request-a", &accepted, '3', 71_000);
    let staged = staged("request-a", &accepted, &claim, 72_000);
    let allocation = allocation("request-a", &staged, 73_000);
    let lease = lease("request-a", &claim, &allocation, 74_000);
    let owner = runtime_owner("request-a", &lease, 75_000);
    let correct = explicit_release(
        "request-a",
        vec![
            claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        76_000,
    );
    reduce(
        &[
            accepted.clone(),
            claim.clone(),
            staged.clone(),
            allocation.clone(),
            lease.clone(),
            owner.clone(),
            correct,
        ],
        &BTreeMap::new(),
    )
    .expect("ordered current sources");

    let out_of_order = explicit_release(
        "request-a",
        vec![
            lease.event_id.clone(),
            claim.event_id.clone(),
            owner.event_id.clone(),
        ],
        77_000,
    );
    assert!(reduce(
        &[
            accepted.clone(),
            claim.clone(),
            staged.clone(),
            allocation.clone(),
            lease.clone(),
            owner.clone(),
            out_of_order,
        ],
        &BTreeMap::new(),
    )
    .is_err());

    let renewed = claim_renewed("request-a", &claim, 78_000);
    let stale = explicit_release(
        "request-a",
        vec![
            claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        79_000,
    );
    assert!(reduce(
        &[
            accepted.clone(),
            claim.clone(),
            staged.clone(),
            allocation.clone(),
            lease.clone(),
            owner.clone(),
            renewed.clone(),
            stale,
        ],
        &BTreeMap::new(),
    )
    .is_err());

    let current = explicit_release(
        "request-a",
        vec![
            renewed.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        80_000,
    );
    reduce(
        &[
            accepted, allocation, claim, lease, owner, renewed, staged, current,
        ],
        &BTreeMap::new(),
    )
    .expect("renewed current source");
}

#[test]
fn runtime_takeover_cannot_move_the_prior_owner_to_another_slot() {
    let accepted = request_accepted("request-takeover", 1_000);
    let claim = request_claim("request-takeover", &accepted, '3', 2_000);
    let staged = staged("request-takeover", &accepted, &claim, 3_000);
    let allocation = allocation("request-takeover", &staged, 4_000);
    let lease = lease("request-takeover", &claim, &allocation, 5_000);
    let owner = runtime_owner("request-takeover", &lease, 6_000);
    let release = explicit_release(
        "request-takeover",
        vec![
            claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        307_000,
    );
    let release_id = release.payload["releaseId"]
        .as_str()
        .expect("release id")
        .to_string();
    let preserved = event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseEvidencePreserved,
        308_000,
        json!({
            "releaseId":release_id,
            "evidenceManifestPath":"requests/request-takeover/evidence-manifest.json",
            "evidenceManifestSha256":h('f'),
            "preservedAtMs":308000
        }),
        Some(release.event_id.clone()),
        Some("request-takeover"),
        vec![release.event_id.clone()],
    );
    let prior_owner_id = owner.payload["runtimeOwnerId"]
        .as_str()
        .expect("owner id")
        .to_string();
    let new_owner_id = format!("owner_{}", "8".repeat(64));
    let takeover = event(
        AggregateKind::RuntimeOwner,
        &new_owner_id,
        EventType::RuntimeTakeoverProven,
        336_001,
        json!({
            "releaseId":release_id,
            "slotId":"slot-02",
            "priorOwnerId":prior_owner_id,
            "priorGeneration":1,
            "newOwnerId":new_owner_id,
            "newGeneration":2,
            "deadOwnerProof":{
                "priorOwnerId":prior_owner_id,
                "priorGeneration":1,
                "expiredAtMs":306000,
                "graceSatisfiedAtMs":336000,
                "processAbsent":true,
                "containerLabelOwnerId":null,
                "containerLabelGeneration":null,
                "leaseInactive":true,
                "claimInactive":true,
                "evidenceRefs":[evidence("dead-owner.json")],
                "provenAtMs":336001
            },
            "provenAtMs":336001
        }),
        None,
        Some("request-takeover"),
        vec![owner.event_id.clone(), preserved.event_id.clone()],
    );
    let error = reduce(
        &[
            accepted, claim, staged, allocation, lease, owner, release, preserved, takeover,
        ],
        &BTreeMap::new(),
    )
    .expect_err("takeover is bound to the prior owner's slot");
    assert!(error.to_string().contains("takeover slot binding"));
}

#[test]
fn runtime_stop_failure_cannot_become_allocatable() {
    let accepted = request_accepted("request-a", 90_000);
    let claim = request_claim("request-a", &accepted, '3', 91_000);
    let staged = staged("request-a", &accepted, &claim, 92_000);
    let allocation = allocation("request-a", &staged, 93_000);
    let lease = lease("request-a", &claim, &allocation, 94_000);
    let owner = runtime_owner("request-a", &lease, 95_000);
    let probe = event(
        AggregateKind::Slot,
        "slot-01",
        EventType::SlotHealthProbeStarted,
        96_000,
        json!({
            "slotId":"slot-01","probeId":"op-health","dockerStatus":"running",
            "deadlineMs":15000,"retryIndex":0,"startedAtMs":96000
        }),
        None,
        Some("request-a"),
        vec![lease.event_id.clone(), owner.event_id.clone()],
    );
    let health = event(
        AggregateKind::Slot,
        "slot-01",
        EventType::SlotHealthObserved,
        97_000,
        json!({
            "slotId":"slot-01","probeId":"op-health","healthStatus":"ready",
            "dockerStatus":"running","cooldownMs":0,"allocatable":true,
            "evidenceRefs":[evidence("health.json")],"observedAtMs":97000
        }),
        Some(probe.event_id.clone()),
        Some("request-a"),
        vec![probe.event_id.clone()],
    );
    let release = explicit_release(
        "request-a",
        vec![
            claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        98_000,
    );
    let release_id = release.payload["releaseId"]
        .as_str()
        .expect("release id")
        .to_string();
    let evidence_preserved = event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseEvidencePreserved,
        99_000,
        json!({
            "releaseId":release_id,"evidenceManifestPath":"requests/request-a/evidence-manifest.json",
            "evidenceManifestSha256":h('f'),"preservedAtMs":99000
        }),
        Some(release.event_id.clone()),
        Some("request-a"),
        vec![release.event_id.clone()],
    );
    let stop_started = event(
        AggregateKind::Release,
        &release_id,
        EventType::RuntimeStopStarted,
        100_000,
        json!({
            "releaseId":release_id,"runtimeOwnerId":owner.payload["runtimeOwnerId"],
            "ownerGeneration":1,"stopTimeoutMs":30000,"startedAtMs":100000
        }),
        Some(evidence_preserved.event_id.clone()),
        Some("request-a"),
        vec![],
    );
    let stop_failed = event(
        AggregateKind::Release,
        &release_id,
        EventType::RuntimeStopFailed,
        101_000,
        json!({
            "releaseId":release_id,"runtimeOwnerId":owner.payload["runtimeOwnerId"],
            "ownerGeneration":1,"dockerStatus":"running","failureReceipt":evidence("stop-failed.json"),
            "reason":"runtime.stop_failed","failedAtMs":101000
        }),
        Some(stop_started.event_id.clone()),
        Some("request-a"),
        vec![stop_started.event_id.clone()],
    );
    let cleanup = event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseCleanupStarted,
        102_000,
        json!({"releaseId":release_id,"startedAtMs":102000}),
        Some(stop_failed.event_id.clone()),
        Some("request-a"),
        vec![stop_failed.event_id.clone()],
    );
    let claim_released = event(
        AggregateKind::Claim,
        claim.payload["claimId"].as_str().expect("claim id"),
        EventType::RequestClaimReleased,
        103_000,
        json!({
            "claimId":claim.payload["claimId"],"claimGeneration":1,
            "releaseId":release_id,"releasedAtMs":103000
        }),
        Some(claim.event_id.clone()),
        Some("request-a"),
        vec![cleanup.event_id.clone()],
    );
    let lease_released = event(
        AggregateKind::Lease,
        lease.payload["leaseId"].as_str().expect("lease id"),
        EventType::SlotLeaseReleased,
        104_000,
        json!({
            "leaseId":lease.payload["leaseId"],"leaseGeneration":1,
            "releaseId":release_id,"releasedAtMs":104000
        }),
        Some(lease.event_id.clone()),
        Some("request-a"),
        vec![claim_released.event_id.clone()],
    );
    let owner_released = event(
        AggregateKind::RuntimeOwner,
        owner.payload["runtimeOwnerId"].as_str().expect("owner id"),
        EventType::RuntimeOwnershipReleased,
        105_000,
        json!({
            "runtimeOwnerId":owner.payload["runtimeOwnerId"],"ownerGeneration":1,
            "releaseId":release_id,"runtimeOutcome":"failed","releasedAtMs":105000
        }),
        Some(owner.event_id.clone()),
        Some("request-a"),
        vec![
            lease_released.event_id.clone(),
            stop_failed.event_id.clone(),
        ],
    );
    let committed = event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseCleanupCommitted,
        106_000,
        json!({
            "releaseId":release_id,"requestClaimReleaseMode":"released",
            "sessionClaimReleaseMode":"not_applicable","leaseReleaseMode":"released",
            "ownerReleaseMode":"released","committedAtMs":106000
        }),
        Some(cleanup.event_id.clone()),
        Some("request-a"),
        vec![
            claim_released.event_id.clone(),
            lease_released.event_id.clone(),
            owner_released.event_id.clone(),
        ],
    );
    let prefix = vec![
        accepted,
        claim,
        staged,
        allocation,
        lease,
        owner,
        probe,
        health.clone(),
        release,
        evidence_preserved,
        stop_started,
        stop_failed,
        cleanup,
        claim_released,
        lease_released,
        owner_released,
        committed.clone(),
    ];
    let invalid_standby = event(
        AggregateKind::Slot,
        "slot-01",
        EventType::SlotStandbyWritten,
        107_000,
        json!({
            "slotId":"slot-01","releaseId":release_id,"allocatable":true,
            "cooldownUntilMs":null,"writtenAtMs":107000
        }),
        Some(health.event_id.clone()),
        Some("request-a"),
        vec![committed.event_id.clone()],
    );
    let mut invalid_standby_events = prefix.clone();
    invalid_standby_events.push(invalid_standby);
    assert!(reduce(&invalid_standby_events, &BTreeMap::new()).is_err());

    let standby = event(
        AggregateKind::Slot,
        "slot-01",
        EventType::SlotStandbyWritten,
        108_000,
        json!({
            "slotId":"slot-01","releaseId":release_id,"allocatable":false,
            "cooldownUntilMs":null,"writtenAtMs":108000
        }),
        Some(health.event_id),
        Some("request-a"),
        vec![committed.event_id.clone()],
    );
    let finalized = event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseFinalized,
        109_000,
        json!({
            "releaseId":release_id,"finalStatus":"allocatable",
            "allocatable":true,"finalizedAtMs":109000
        }),
        Some(committed.event_id),
        Some("request-a"),
        vec![standby.event_id.clone()],
    );
    let mut invalid_final_events = prefix;
    invalid_final_events.extend([standby, finalized]);
    assert!(reduce(&invalid_final_events, &BTreeMap::new()).is_err());
}
