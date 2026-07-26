use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::claims::fencing_hash;
use gpt_webai_lifecycle::contracts::events::{
    Aggregate, AggregateKind, EventEnvelope, EventType, Writer,
};
use gpt_webai_lifecycle::contracts::ids::h256;
use gpt_webai_lifecycle::journal::{EventStore, HeadStore};
use gpt_webai_lifecycle::sessions::{new_session_record, write_session_record, NewSessionRecord};
use serde_json::{json, Value};

pub(super) fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gpt-webai-lifecycle"))
}

pub(super) fn stdout_json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).expect("stdout json")
}

pub(super) struct Fixture {
    pub root: PathBuf,
}

impl Fixture {
    pub(super) fn new(prefix: &str) -> Self {
        let root = temp_state_root(prefix);
        Self { root }
    }

    pub(super) fn write_session(&self, session_id: &str, slot_id: &str, _account_group: &str) {
        let record = new_session_record(NewSessionRecord {
            request_id: Some(format!("request-{session_id}")),
            run_id: Some("run-release".to_string()),
            session_id: session_id.to_string(),
            conversation_url: format!("https://chatgpt.com/c/{session_id}"),
            slot_id: slot_id.to_string(),
            cohort: gpt_webai_lifecycle::allocator::cohort_of(slot_id)
                .expect("fixture slot cohort")
                .to_string(),
            page_binding_generation: 1,
        })
        .expect("new session");
        write_session_record(&self.root, &record).expect("write session");
    }

    pub(super) fn seed_active_session(&self, session_id: &str, fencing_token: &str) {
        self.write_session(session_id, "slot-01", "cohort-a");
        let request_id = format!("request-{session_id}");
        let at = now_ms();
        let claim_id = format!("claim_{}", "1".repeat(64));
        let claim = event(
            AggregateKind::Claim,
            &claim_id,
            EventType::SessionOperationClaimGranted,
            at,
            json!({
                "claimId":claim_id,"sessionId":session_id,"operationKind":"show",
                "expectedSlotId":"slot-01","expectedCohort":"cohort-a",
                "expectedRuntimeOwnerGeneration":null,"requestId":request_id,
                "runId":"run-release","ttlMs":300000,"grantedAtMs":at,
                "renewAtMs":at+100000,"expiresAtMs":at+300000,
                "fencingTokenSha256":fencing_hash(fencing_token)
            }),
            None,
            Some(&request_id),
            vec![],
        );
        let lease_id = format!("lease_{}", "2".repeat(64));
        let lease = event(
            AggregateKind::Lease,
            &lease_id,
            EventType::PersistedSessionLeaseGranted,
            at + 1,
            json!({
                "leaseId":lease_id,"claimId":claim_id,"slotId":"slot-01",
                "cohort":"cohort-a","leaseGeneration":1,"reason":"persisted_session",
                "grantedAtMs":at+1,"renewAtMs":at+100001,
                "expiresAtMs":at+300001,"fencingTokenSha256":fencing_hash(fencing_token)
            }),
            None,
            Some(&request_id),
            vec![claim.event_id.clone()],
        );
        let owner_id = format!("owner_{}", "3".repeat(64));
        let owner = event(
            AggregateKind::RuntimeOwner,
            &owner_id,
            EventType::SessionRuntimeOwnershipGranted,
            at + 2,
            json!({
                "runtimeOwnerId":owner_id,"sessionId":session_id,
                "slotId":"slot-01","leaseId":lease_id,
                "ownerGeneration":1,"runtimeIncarnationId":format!("runtime_{}","4".repeat(64)),
                "dockerStatus":"running","startReceipt":evidence("runtime-start.receipt.json"),
                "grantedAtMs":at+2,"renewAtMs":at+100002,"expiresAtMs":at+300002,
                "fencingTokenSha256":fencing_hash(fencing_token)
            }),
            None,
            Some(&request_id),
            vec![claim.event_id.clone(), lease.event_id.clone()],
        );
        let probe = event(
            AggregateKind::Slot,
            "slot-01",
            EventType::SlotHealthProbeStarted,
            at + 3,
            json!({
                "slotId":"slot-01","probeId":"op-release-probe","dockerStatus":"running",
                "deadlineMs":15000,"retryIndex":0,"startedAtMs":at+3
            }),
            None,
            Some(&request_id),
            vec![lease.event_id.clone(), owner.event_id.clone()],
        );
        let health = event(
            AggregateKind::Slot,
            "slot-01",
            EventType::SlotHealthObserved,
            at + 4,
            json!({
                "slotId":"slot-01","probeId":"op-release-probe","healthStatus":"ready",
                "dockerStatus":"running","cooldownMs":0,"allocatable":false,
                "evidenceRefs":[],"observedAtMs":at+4
            }),
            Some(probe.event_id.clone()),
            Some(&request_id),
            vec![probe.event_id.clone()],
        );
        let events = [claim, lease, owner, probe, health];
        let head = HeadStore::new(&self.root);
        let guard = head.acquire_mutation().expect("mutation guard");
        EventStore::new(&self.root)
            .append_transaction(&guard, &events)
            .expect("seed R13 state");
    }

    pub(super) fn write_fake_docker(&self, log_path: &Path, exit_code: u8) -> PathBuf {
        let path = self.root.join(format!("fake-docker-{exit_code}.sh"));
        std::fs::write(
            &path,
            format!(r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> '{}'
if [[ "$1" == "inspect" ]]; then
  printf '%s\n' '[{{"Id":"{}","Name":"/gpt-webai-slot-01","Config":{{"Labels":{{"pr72.gpt-webai.owner-id":"owner_{}","pr72.gpt-webai.owner-generation":"1","pr72.gpt-webai.runtime-incarnation":"runtime_{}"}}}},"State":{{"Status":"exited","StartedAt":"2026-07-24T00:00:00Z","FinishedAt":"2026-07-24T00:00:01Z","ExitCode":0}}}}]'
  exit 0
fi
exit {exit_code}
"#, log_path.display(), "a".repeat(64), "3".repeat(64), "4".repeat(64)),
        )
        .expect("write fake docker");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("chmod");
        path
    }
}

#[allow(clippy::too_many_arguments)]
fn event(
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
        "op-seed-release".to_string(),
        payload,
        predecessor,
        request_id.map(str::to_string),
        request_id.map(|_| "run-release".to_string()),
        sources,
        Writer {
            host_id: format!("host_{}", "1".repeat(32)),
            process_id: std::process::id(),
            process_start_ms: at_ms,
            writer_id: format!("writer_{}", "5".repeat(64)),
        },
    )
    .expect("fixture event")
}

fn evidence(name: &str) -> Value {
    json!({
        "path":format!("evidence/requests/r-request-release/operations/op-seed/{name}"),
        "sha256":h256(b"evidence"),"sizeBytes":1,"mediaType":"application/json"
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temp_state_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gpt-webai-cli-release-{name}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create state root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("private state root");
    root
}
