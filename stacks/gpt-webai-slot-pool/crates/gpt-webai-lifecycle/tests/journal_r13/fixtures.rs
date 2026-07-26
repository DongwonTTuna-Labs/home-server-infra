#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gpt_webai_lifecycle::contracts::events::{
    Aggregate, AggregateKind, EventEnvelope, EventType, Writer,
};
use gpt_webai_lifecycle::uploads::AttachmentSet;
use serde_json::{json, Value};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pr72-journal-r13-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("create private temp root");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn writer() -> Writer {
    Writer {
        host_id: format!("host_{}", "1".repeat(32)),
        process_id: 7,
        process_start_ms: 1_000,
        writer_id: format!("writer_{}", "2".repeat(64)),
    }
}

pub fn request_accepted(request_id: &str, at_ms: u64) -> EventEnvelope {
    event(
        AggregateKind::Request,
        request_id,
        EventType::RequestAccepted,
        at_ms,
        json!({
            "requestId": request_id,
            "kind": "pro",
            "promptSha256": h('a'),
            "promptSizeBytes": 5,
            "attachmentCount": 0,
            "artifactExpectation": "optional",
            "acceptedAtMs": at_ms
        }),
        None,
        Some(request_id),
        vec![],
    )
}

pub fn request_claim(
    request_id: &str,
    source: &EventEnvelope,
    digit: char,
    at_ms: u64,
) -> EventEnvelope {
    let claim_id = format!("claim_{}", digit.to_string().repeat(64));
    event(
        AggregateKind::Claim,
        &claim_id,
        EventType::RequestClaimGranted,
        at_ms,
        json!({
            "claimId": claim_id,
            "requestId": request_id,
            "claimGeneration": 1,
            "ttlMs": 300000,
            "grantedAtMs": at_ms,
            "renewAtMs": at_ms + 100000,
            "expiresAtMs": at_ms + 300000,
            "fencingTokenSha256": h('b')
        }),
        None,
        Some(request_id),
        vec![source.event_id.clone()],
    )
}

pub fn staged(
    request_id: &str,
    accepted: &EventEnvelope,
    claim: &EventEnvelope,
    at_ms: u64,
) -> EventEnvelope {
    let attachment_set = AttachmentSet::from_records(Vec::new()).expect("empty attachment set");
    event(
        AggregateKind::Request,
        request_id,
        EventType::HostAttachmentsStaged,
        at_ms,
        json!({"requestId":request_id,"attachmentSet":attachment_set,"stagedAtMs":at_ms}),
        Some(accepted.event_id.clone()),
        Some(request_id),
        vec![claim.event_id.clone()],
    )
}

pub fn allocation(request_id: &str, staged: &EventEnvelope, at_ms: u64) -> EventEnvelope {
    event(
        AggregateKind::Allocator,
        "allocator",
        EventType::AllocationCandidateObserved,
        at_ms,
        json!({
            "requestId":request_id,"scanOrdinal":0,"cohort":"cohort-a","slotId":"slot-01",
            "cohortCursorBefore":0,"withinCursorBefore":0,"decision":"grantable",
            "skipReason":null,"observedAtMs":at_ms
        }),
        None,
        Some(request_id),
        vec![staged.event_id.clone()],
    )
}

pub fn lease(
    request_id: &str,
    claim: &EventEnvelope,
    allocation: &EventEnvelope,
    at_ms: u64,
) -> EventEnvelope {
    let lease_id = format!("lease_{}", "4".repeat(64));
    event(
        AggregateKind::Lease,
        &lease_id,
        EventType::SlotLeaseGranted,
        at_ms,
        json!({
            "leaseId":lease_id,"claimId":claim.payload["claimId"],"slotId":"slot-01",
            "cohort":"cohort-a","cohortCursorBefore":0,"withinCursorBefore":0,
            "cohortCursorAfter":1,"withinCursorAfter":1,"leaseGeneration":1,
            "reason":"fresh_send","grantedAtMs":at_ms,"renewAtMs":at_ms+100000,
            "expiresAtMs":at_ms+300000,"fencingTokenSha256":h('c')
        }),
        None,
        Some(request_id),
        vec![claim.event_id.clone(), allocation.event_id.clone()],
    )
}

pub fn claim_renewed(request_id: &str, claim: &EventEnvelope, at_ms: u64) -> EventEnvelope {
    let claim_id = claim.payload["claimId"].as_str().expect("claim id");
    event(
        AggregateKind::Claim,
        claim_id,
        EventType::RequestClaimRenewed,
        at_ms,
        json!({
            "claimId":claim_id,"claimGeneration":1,"renewalRevision":2,
            "renewedAtMs":at_ms,"renewAtMs":at_ms+100000,"expiresAtMs":at_ms+300000
        }),
        Some(claim.event_id.clone()),
        Some(request_id),
        vec![claim.event_id.clone()],
    )
}

pub fn runtime_owner(request_id: &str, lease: &EventEnvelope, at_ms: u64) -> EventEnvelope {
    let owner_id = format!("owner_{}", "5".repeat(64));
    event(
        AggregateKind::RuntimeOwner,
        &owner_id,
        EventType::RuntimeOwnershipGranted,
        at_ms,
        json!({
            "runtimeOwnerId":owner_id,"slotId":"slot-01","leaseId":lease.payload["leaseId"],
            "ownerGeneration":1,"runtimeIncarnationId":format!("runtime_{}", "6".repeat(64)),
            "dockerStatus":"running","startReceipt":evidence("runtime-start.json"),
            "grantedAtMs":at_ms,"renewAtMs":at_ms+100000,"expiresAtMs":at_ms+300000,
            "fencingTokenSha256":h('d')
        }),
        None,
        Some(request_id),
        vec![lease.event_id.clone()],
    )
}

pub fn explicit_release(request_id: &str, sources: Vec<String>, at_ms: u64) -> EventEnvelope {
    let release_id = format!("release_{}", "7".repeat(64));
    event(
        AggregateKind::Release,
        &release_id,
        EventType::ReleaseStarted,
        at_ms,
        json!({
            "releaseId":release_id,"subjectKind":"request","subjectId":request_id,
            "reason":"release.explicit","startedAtMs":at_ms
        }),
        None,
        Some(request_id),
        sources,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn event(
    kind: AggregateKind,
    aggregate_id: &str,
    event_type: EventType,
    at_ms: u64,
    payload: Value,
    predecessor: Option<String>,
    request_id: Option<&str>,
    sources: Vec<String>,
) -> EventEnvelope {
    EventEnvelope::create(
        Aggregate {
            id: aggregate_id.to_string(),
            kind,
        },
        at_ms,
        event_type,
        format!("op-{at_ms}"),
        payload,
        predecessor,
        request_id.map(ToString::to_string),
        request_id.map(|value| format!("run-{value}")),
        sources,
        writer(),
    )
    .expect("valid fixture event")
}

pub fn h(digit: char) -> String {
    format!("sha256:{}", digit.to_string().repeat(64))
}

pub fn evidence(name: &str) -> Value {
    json!({
        "path":format!("requests/request-a/operations/{name}"),
        "sha256":h('e'),
        "sizeBytes":1,
        "mediaType":"application/json"
    })
}
