use std::ffi::OsStr;
use std::fs;

use gpt_webai_lifecycle::claims::fencing_hash;
use gpt_webai_lifecycle::config::{
    load_or_create_host_id, resolve_host_id_seed_path, resolve_state_root,
};
use gpt_webai_lifecycle::contracts::browser::{EvidenceMediaType, EvidenceRef};
use gpt_webai_lifecycle::contracts::events::Writer;
use gpt_webai_lifecycle::contracts::ids::{derive_runtime_incarnation_id, h256};
use gpt_webai_lifecycle::contracts::projection::RuntimeOwnerRecord;
use gpt_webai_lifecycle::runtime::ownership::{
    adopt_ownership, current_owner_can_stop, generate_incarnation_nonce, grant_ownership,
    process_absent_from_observation, process_start_ms_from_proc, takeover, validate_dead_owner,
    AdoptionProof, DeadOwnerProof, OwnershipGrant,
};
use gpt_webai_lifecycle::runtime::{
    parse_docker_inspect, write_runtime_adoption_evidence, write_runtime_start_evidence,
    write_runtime_stop_evidence, RuntimeAdoptionReceipt, RuntimeReceiptLabels, RuntimeStartReceipt,
    RuntimeStopReceipt,
};

#[test]
fn grant_persists_exact_fence_and_runtime_identity() {
    let record = owner("token", 1_000);
    assert_eq!(record.cas.kind, "runtime_owner");
    assert_eq!(record.cas.subject_id, "slot-01");
    assert_eq!(record.cas.generation, 1);
    assert_eq!(record.cas.renewal_revision, 1);
    assert_eq!(record.cas.fencing_token_sha256, Some(fencing_hash("token")));
    assert_eq!(record.cas.renew_at_ms, 101_000);
    assert_eq!(record.cas.expires_at_ms, 301_000);
    assert_eq!(record.runtime_incarnation_id, runtime_id());
    assert_eq!(record.docker_status, "running");
}

#[test]
fn current_owner_stop_requires_exact_generation_token_and_live_deadline() {
    let record = owner("token", 1_000);
    assert!(current_owner_can_stop(&record, 1, "token", 300_999));
    assert!(!current_owner_can_stop(&record, 2, "token", 2_000));
    assert!(!current_owner_can_stop(&record, 1, "wrong", 2_000));
    assert!(!current_owner_can_stop(&record, 1, "token", 301_000));
}

#[test]
fn dead_owner_proof_is_closed_over_identity_grace_and_inactive_resources() {
    let record = owner("token", 1_000);
    let proof = proof(&record);
    validate_dead_owner(&record, &proof).expect("complete proof");

    let mut wrong_generation = proof.clone();
    wrong_generation.prior_generation = 2;
    assert!(validate_dead_owner(&record, &wrong_generation).is_err());

    let mut early = proof.clone();
    early.grace_satisfied_at_ms -= 1;
    assert!(validate_dead_owner(&record, &early).is_err());

    let mut process_present = proof.clone();
    process_present.process_absent = false;
    assert!(validate_dead_owner(&record, &process_present).is_err());

    let mut lease_active = proof.clone();
    lease_active.lease_inactive = false;
    assert!(validate_dead_owner(&record, &lease_active).is_err());

    let mut claim_active = proof.clone();
    claim_active.claim_inactive = false;
    assert!(validate_dead_owner(&record, &claim_active).is_err());

    let mut no_evidence = proof.clone();
    no_evidence.evidence_refs.clear();
    assert!(validate_dead_owner(&record, &no_evidence).is_err());

    let mut other_live_label = proof;
    other_live_label.container_label_owner_id = Some(format!("owner_{}", "9".repeat(64)));
    other_live_label.container_label_generation = Some(1);
    assert!(validate_dead_owner(&record, &other_live_label).is_err());
}

#[test]
fn exited_container_birth_labels_do_not_block_takeover() {
    let mut record = owner("token", 1_000);
    record.docker_status = "exited".to_string();
    let mut stopped_label = proof(&record);
    stopped_label.container_label_owner_id = Some(format!("owner_{}", "9".repeat(64)));
    stopped_label.container_label_generation = Some(7);
    validate_dead_owner(&record, &stopped_label).expect("exited labels are birth evidence only");
}

#[test]
fn state_paths_host_seed_and_procfs_sources_follow_r23() {
    assert_eq!(
        resolve_state_root(
            None,
            Some(OsStr::new("/state")),
            Some(OsStr::new("/home/user")),
        )
        .unwrap(),
        std::path::PathBuf::from("/state/gpt-webai-lifecycle/r13")
    );
    assert_eq!(
        resolve_state_root(
            Some(OsStr::new("/fixture/root")),
            Some(OsStr::new("/state")),
            None,
        )
        .unwrap(),
        std::path::PathBuf::from("/fixture/root")
    );
    assert_eq!(
        resolve_host_id_seed_path(None, Some(OsStr::new("/home/user"))).unwrap(),
        std::path::PathBuf::from("/home/user/.local/state/gpt-webai-lifecycle/host-id")
    );

    let root = temp_root("host-id");
    let seed_path = root.join("host-id");
    let first = load_or_create_host_id(&seed_path).expect("create host id");
    let second = load_or_create_host_id(&seed_path).expect("reuse host id");
    assert_eq!(first, second);
    assert_eq!(fs::read(&seed_path).expect("seed bytes").len(), 33);
    fs::write(&seed_path, b"corrupt\n").expect("corrupt seed");
    let repaired = load_or_create_host_id(&seed_path).expect("repair host id");
    assert_ne!(repaired, first);
    assert_eq!(fs::read(&seed_path).expect("repaired bytes").len(), 33);
    fs::remove_dir_all(root).expect("cleanup temp root");

    let tail = std::iter::once("S".to_string())
        .chain((0..18).map(|_| "0".to_string()))
        .chain(std::iter::once("250".to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        process_start_ms_from_proc(
            &format!("42 (worker with spaces) {tail}"),
            "cpu 1 2 3\nbtime 1000\n",
            100,
        )
        .unwrap(),
        1_002_500
    );
}

#[test]
fn process_absence_is_host_and_pid_reuse_safe() {
    let writer = writer();
    assert!(!process_absent_from_observation(
        &writer,
        &format!("host_{}", "9".repeat(32)),
        None,
    ));
    assert!(process_absent_from_observation(
        &writer,
        &writer.host_id,
        None,
    ));
    assert!(!process_absent_from_observation(
        &writer,
        &writer.host_id,
        Some(writer.process_start_ms),
    ));
    assert!(process_absent_from_observation(
        &writer,
        &writer.host_id,
        Some(writer.process_start_ms + 1),
    ));
}

#[test]
fn takeover_retires_prior_and_creates_tokenless_next_generation() {
    let record = owner("token", 1_000);
    let proof = proof(&record);
    let takeover_event_id = event_id('8');
    let (retired, replacement) = takeover(
        &record,
        &proof,
        &format!("release_{}", "7".repeat(64)),
        writer(),
        takeover_event_id.clone(),
    )
    .expect("takeover");
    assert_eq!(retired.cas.status, "released");
    assert_eq!(retired.cas.released_at_ms, Some(proof.proven_at_ms));
    assert_eq!(retired.cas.release_event_id, Some(takeover_event_id));
    assert_eq!(replacement.cas.subject_id, record.cas.subject_id);
    assert_eq!(replacement.cas.generation, 2);
    assert_eq!(replacement.cas.renewal_revision, 1);
    assert!(replacement.cas.fencing_token_sha256.is_none());
    assert_eq!(
        replacement.runtime_incarnation_id,
        record.runtime_incarnation_id
    );
    assert_eq!(replacement.docker_status, record.docker_status);
}

#[test]
fn adoption_rejects_container_labels_for_another_owner() {
    let error = adopt_ownership(
        grant("token", 1_000),
        &AdoptionProof {
            container_label_owner_id: Some(format!("owner_{}", "9".repeat(64))),
            container_label_generation: Some(1),
            observed_docker_status: "running".to_string(),
        },
    )
    .expect_err("labels must bind to the granted owner");
    assert!(error.to_string().contains("AdoptionProof identity"));
}

#[test]
fn docker_inspect_writes_closed_start_receipt_and_is_idempotent() {
    let root = temp_root("runtime-start-receipt");
    let evidence_root = root.join("evidence/requests/r-request/operations/op-runtime-start");
    let container_id = "a".repeat(64);
    let started_at = "2026-07-24T12:34:56.123456789Z";
    let incarnation = derive_runtime_incarnation_id("slot-01", "0123456789abcdef0123456789abcdef")
        .expect("incarnation");
    let expected_labels = RuntimeReceiptLabels {
        owner_id: format!("owner_{}", "3".repeat(64)),
        owner_generation: 7,
        runtime_incarnation_id: incarnation.clone(),
    };
    let inspect = docker_inspect_bytes(
        &container_id,
        "running",
        started_at,
        None,
        Some(0),
        Some(&format!("owner_{}", "3".repeat(64))),
        Some(7),
        Some(&incarnation),
    );

    let first = write_runtime_start_evidence(
        &root,
        &evidence_root,
        "slot-01",
        &expected_labels,
        &inspect,
        10_000,
    )
    .expect("write start evidence");
    let second = write_runtime_start_evidence(
        &root,
        &evidence_root,
        "slot-01",
        &expected_labels,
        &inspect,
        10_000,
    )
    .expect("idempotent rewrite");
    assert_eq!(first, second);
    assert_eq!(first.media_type, EvidenceMediaType::Json);
    assert_eq!(
        first.path,
        "evidence/requests/r-request/operations/op-runtime-start/runtime-start.receipt.json"
    );

    let receipt_bytes = fs::read(root.join(&first.path)).expect("start receipt bytes");
    assert_eq!(first.sha256, h256(&receipt_bytes));
    assert_eq!(first.size_bytes, receipt_bytes.len() as u64);
    let receipt: RuntimeStartReceipt =
        serde_json::from_slice(&receipt_bytes).expect("closed start receipt");
    assert_eq!(receipt.schema_version, "pr72.runtime-start-receipt.r13.v1");
    assert_eq!(receipt.slot_id, "slot-01");
    assert_eq!(receipt.container_id, container_id);
    assert_eq!(receipt.container_name, "/gpt-webai-slot-01");
    assert_eq!(receipt.docker_status, "running");
    assert_eq!(receipt.container_started_at, started_at);
    assert_eq!(receipt.labels.owner_generation, 7);
    assert_eq!(receipt.labels.runtime_incarnation_id, incarnation);
    assert_eq!(receipt.inspect_sha256, h256(&inspect));
    assert_eq!(
        fs::read(evidence_root.join("docker-inspect.json")).expect("raw inspect"),
        inspect
    );

    let collision = write_runtime_start_evidence(
        &root,
        &evidence_root,
        "slot-01",
        &expected_labels,
        &docker_inspect_bytes(
            &"b".repeat(64),
            "running",
            started_at,
            None,
            Some(0),
            Some(&format!("owner_{}", "3".repeat(64))),
            Some(7),
            Some(&incarnation),
        ),
        10_000,
    )
    .expect_err("different immutable inspect must collide");
    assert!(collision.to_string().contains("immutable collision"));
    if root.exists() {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

#[test]
fn stop_and_adoption_receipts_preserve_nullable_docker_evidence() {
    let root = temp_root("runtime-stop-adoption-receipts");
    let stop_root = root.join("evidence/requests/r-request/operations/op-runtime-stop");
    let adoption_root = root.join("evidence/requests/r-request/operations/op-runtime-adopt");
    let container_id = "c".repeat(64);
    let started_at = "2026-07-24T12:34:56Z";
    let finished_at = "2026-07-24T12:35:56Z";
    let incarnation = derive_runtime_incarnation_id("slot-02", "abcdef0123456789abcdef0123456789")
        .expect("incarnation");
    let inspect = docker_inspect_bytes(
        &container_id,
        "exited",
        started_at,
        Some(finished_at),
        Some(137),
        None,
        None,
        Some(&incarnation),
    );

    let stop_ref = write_runtime_stop_evidence(&root, &stop_root, "slot-02", &inspect, 20_000)
        .expect("stop evidence");
    let stop: RuntimeStopReceipt =
        serde_json::from_slice(&fs::read(root.join(stop_ref.path)).expect("stop receipt"))
            .expect("closed stop receipt");
    assert_eq!(stop.schema_version, "pr72.runtime-stop-receipt.r13.v1");
    assert_eq!(stop.docker_status, "exited");
    assert_eq!(stop.container_finished_at.as_deref(), Some(finished_at));
    assert_eq!(stop.exit_code, Some(137));
    assert_eq!(stop.inspect_sha256, h256(&inspect));

    let adoption_ref =
        write_runtime_adoption_evidence(&root, &adoption_root, "slot-02", &inspect, 20_001)
            .expect("adoption evidence");
    let adoption: RuntimeAdoptionReceipt =
        serde_json::from_slice(&fs::read(root.join(adoption_ref.path)).expect("adoption receipt"))
            .expect("closed adoption receipt");
    assert_eq!(
        adoption.schema_version,
        "pr72.runtime-adoption-receipt.r13.v1"
    );
    assert_eq!(adoption.observed_docker_status, "exited");
    assert!(adoption.container_label_owner_id.is_none());
    assert!(adoption.container_label_generation.is_none());
    assert_eq!(
        adoption.container_label_incarnation.as_deref(),
        Some(incarnation.as_str())
    );
    assert_eq!(adoption.inspect_sha256, h256(&inspect));
    fs::remove_dir_all(root).expect("cleanup temp root");
}

#[test]
fn start_receipt_rejects_missing_or_mismatched_birth_labels() {
    let root = temp_root("runtime-start-label-rejection");
    let evidence_root = root.join("evidence/requests/r-request/operations/op-runtime-start");
    let container_id = "d".repeat(64);
    let started_at = "2026-07-24T12:34:56Z";
    let expected_labels = RuntimeReceiptLabels {
        owner_id: format!("owner_{}", "4".repeat(64)),
        owner_generation: 1,
        runtime_incarnation_id: derive_runtime_incarnation_id(
            "slot-03",
            "0123456789abcdef0123456789abcdef",
        )
        .expect("incarnation"),
    };
    let missing = docker_inspect_bytes(
        &container_id,
        "running",
        started_at,
        None,
        Some(0),
        None,
        None,
        None,
    );
    assert!(write_runtime_start_evidence(
        &root,
        &evidence_root,
        "slot-03",
        &expected_labels,
        &missing,
        30_000,
    )
    .is_err());

    let mismatched = docker_inspect_bytes(
        &container_id,
        "running",
        started_at,
        None,
        Some(0),
        Some(&format!("owner_{}", "4".repeat(64))),
        Some(1),
        Some(&format!("runtime_{}", "5".repeat(64))),
    );
    assert!(write_runtime_start_evidence(
        &root,
        &evidence_root,
        "slot-03",
        &expected_labels,
        &mismatched,
        30_000,
    )
    .is_err());
    assert!(!evidence_root.join("runtime-start.receipt.json").exists());
    if root.exists() {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

#[test]
fn runtime_incarnation_nonce_is_os_csprng_lower_hex() {
    let first = generate_incarnation_nonce().expect("first nonce");
    let second = generate_incarnation_nonce().expect("second nonce");
    assert_eq!(first.len(), 32);
    assert!(first
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_ne!(first, second);
}

#[test]
fn adoption_receipt_rejects_zero_generation_and_unsafe_evidence_root() {
    let root = temp_root("runtime-adoption-rejection");
    let container_id = "e".repeat(64);
    let started_at = "2026-07-24T12:34:56Z";
    let invalid_generation = docker_inspect_bytes(
        &container_id,
        "running",
        started_at,
        None,
        Some(0),
        Some(&format!("owner_{}", "6".repeat(64))),
        Some(0),
        None,
    );
    let evidence_root = root.join("evidence/requests/r-request/operations/op-runtime-adopt");
    assert!(write_runtime_adoption_evidence(
        &root,
        &evidence_root,
        "slot-04",
        &invalid_generation,
        40_000,
    )
    .is_err());

    let valid = docker_inspect_bytes(
        &container_id,
        "running",
        started_at,
        None,
        Some(0),
        None,
        None,
        None,
    );
    let outside_name = format!(
        "{}-outside",
        root.file_name()
            .and_then(OsStr::to_str)
            .expect("temp root name")
    );
    let unsafe_root = root.join("..").join(&outside_name);
    assert!(
        write_runtime_adoption_evidence(&root, &unsafe_root, "slot-04", &valid, 40_001,).is_err()
    );
    assert!(!root
        .parent()
        .expect("temp root parent")
        .join(outside_name)
        .exists());
    if root.exists() {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

#[allow(clippy::too_many_arguments)]
fn docker_inspect_bytes(
    container_id: &str,
    status: &str,
    started_at: &str,
    finished_at: Option<&str>,
    exit_code: Option<i64>,
    owner_id: Option<&str>,
    owner_generation: Option<u16>,
    incarnation: Option<&str>,
) -> Vec<u8> {
    let mut labels = serde_json::Map::new();
    if let Some(owner_id) = owner_id {
        labels.insert(
            "pr72.gpt-webai.owner-id".to_string(),
            serde_json::Value::String(owner_id.to_string()),
        );
    }
    if let Some(owner_generation) = owner_generation {
        labels.insert(
            "pr72.gpt-webai.owner-generation".to_string(),
            serde_json::Value::String(owner_generation.to_string()),
        );
    }
    if let Some(incarnation) = incarnation {
        labels.insert(
            "pr72.gpt-webai.runtime-incarnation".to_string(),
            serde_json::Value::String(incarnation.to_string()),
        );
    }
    let value = serde_json::json!([{
        "Config": {"Labels": labels},
        "Id": container_id,
        "Name": "/gpt-webai-slot-01",
        "State": {
            "ExitCode": exit_code,
            "FinishedAt": finished_at,
            "StartedAt": started_at,
            "Status": status,
        }
    }]);
    let bytes = serde_json::to_vec(&value).expect("docker inspect fixture");
    parse_docker_inspect(&bytes).expect("valid inspect fixture");
    bytes
}

fn owner(token: &str, now_ms: u64) -> RuntimeOwnerRecord {
    grant_ownership(grant(token, now_ms)).expect("grant owner")
}

fn grant(token: &str, now_ms: u64) -> OwnershipGrant<'_> {
    OwnershipGrant {
        slot_id: "slot-01",
        operation_id: "op-runtime-owner",
        runtime_incarnation_id:
            "runtime_2222222222222222222222222222222222222222222222222222222222222222",
        docker_status: "running",
        fencing_token: token,
        owner: writer(),
        generation: 1,
        now_ms,
        event_id: event_id('1'),
    }
}

fn proof(record: &RuntimeOwnerRecord) -> DeadOwnerProof {
    DeadOwnerProof {
        prior_owner_id: record.cas.id.clone(),
        prior_generation: record.cas.generation,
        expired_at_ms: record.cas.expires_at_ms,
        grace_satisfied_at_ms: record.cas.expires_at_ms + 30_000,
        process_absent: true,
        container_label_owner_id: None,
        container_label_generation: None,
        lease_inactive: true,
        claim_inactive: true,
        evidence_refs: vec![EvidenceRef {
            path: "requests/request-a/operations/release/dead-owner.json".to_string(),
            sha256: h256(b"dead owner proof"),
            size_bytes: 1,
            media_type: EvidenceMediaType::Json,
        }],
        proven_at_ms: record.cas.expires_at_ms + 30_001,
    }
}

fn writer() -> Writer {
    Writer {
        host_id: format!("host_{}", "1".repeat(32)),
        process_id: 7,
        process_start_ms: 1_000,
        writer_id: format!("writer_{}", "2".repeat(64)),
    }
}

fn runtime_id() -> String {
    format!("runtime_{}", "2".repeat(64))
}

fn event_id(digit: char) -> String {
    format!("evt_{}", digit.to_string().repeat(64))
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "gpt-webai-r23-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos(),
    ))
}
