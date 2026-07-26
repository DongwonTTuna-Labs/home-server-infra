use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpt_webai_lifecycle::artifact_claims::baseline::ArtifactBaseline;
use gpt_webai_lifecycle::artifact_claims::completion::{mime_from_path, PlaywrightDownloadReceipt};
use gpt_webai_lifecycle::artifact_claims::recovery::{ArtifactClaim, ClaimOutcome};
use gpt_webai_lifecycle::artifact_claims::{
    ArtifactControl, ArtifactExpectation, BottomProof, ZeroControlProof,
};
use gpt_webai_lifecycle::contracts::browser::{
    EvidenceMediaType, EvidenceRef, PageBindingEcho, SessionEcho,
};
use gpt_webai_lifecycle::contracts::ids::{artifact_host_saved_rel_path, derive_artifact_id, h256};

#[test]
fn zero_controls_distinguish_optional_and_required_claims() {
    let proof = zero_proof();
    let mut optional = claim(ArtifactExpectation::Optional);
    optional.discover_zero(&proof).unwrap();
    assert_eq!(
        optional.outcome(),
        &ClaimOutcome::ZeroControlsOptionalSuccess
    );

    let mut required = claim(ArtifactExpectation::Required);
    required.discover_zero(&proof).unwrap();
    assert_eq!(
        required.outcome(),
        &ClaimOutcome::Failed("artifact.required_zero")
    );
}

#[test]
fn consumed_attempt_cannot_be_reclicked_and_stable_file_is_diagnostic_only() {
    let mut claim = claim(ArtifactExpectation::Required);
    claim
        .discover_controls(vec![control()], &bottom_proof())
        .unwrap();
    claim
        .consume_next("attempt-1".to_string(), baseline())
        .unwrap();
    assert!(claim
        .consume_next("attempt-2".to_string(), baseline())
        .is_err());
    claim
        .observe_recovery_candidate("artifacts/candidate.zip", &h256(b"candidate"), 2)
        .unwrap();
    assert_eq!(claim.outcome(), &ClaimOutcome::Pending);
    assert!(claim.consumed().is_some());
}

#[test]
fn real_playwright_receipt_and_reopened_bytes_complete_claim() {
    let fixture = Fixture::new();
    let receipt = download_receipt(b"zip-bytes");
    let rel = &receipt.host_saved_rel_path;
    let path = fixture.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"zip-bytes").unwrap();

    let mut claim = claim(ArtifactExpectation::Required);
    claim
        .discover_controls(vec![control()], &bottom_proof())
        .unwrap();
    claim
        .consume_next("attempt-1".to_string(), baseline())
        .unwrap();
    claim
        .complete_consumed(&session_echo(), receipt, fixture.path())
        .unwrap();
    assert_eq!(claim.outcome(), &ClaimOutcome::Downloaded);
}

#[test]
fn file_appearance_without_event_or_zero_byte_never_completes() {
    let fixture = Fixture::new();
    let mut receipt = download_receipt(b"");
    let rel = &receipt.host_saved_rel_path;
    let path = fixture.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"").unwrap();
    receipt.download_event_id = "missing-event".to_string();

    let mut claim = claim(ArtifactExpectation::Required);
    claim
        .discover_controls(vec![control()], &bottom_proof())
        .unwrap();
    claim
        .consume_next("attempt-1".to_string(), baseline())
        .unwrap();
    assert!(claim
        .complete_consumed(&session_echo(), receipt, fixture.path())
        .is_err());
    assert!(claim.consumed().is_some());
}

#[test]
fn mime_oracle_is_extension_only_and_closed() {
    assert_eq!(mime_from_path("file.json"), "application/json");
    assert_eq!(mime_from_path("file.jpeg"), "image/jpeg");
    assert_eq!(mime_from_path("file.unknown"), "application/octet-stream");
    assert_eq!(mime_from_path("file.JSON"), "application/octet-stream");
}

#[test]
fn duplicate_or_prior_turn_controls_are_rejected() {
    let mut duplicate_claim = claim(ArtifactExpectation::Required);
    assert!(duplicate_claim
        .discover_controls(vec![control(), control()], &bottom_proof())
        .is_err());
    let mut wrong_turn = control();
    wrong_turn.current_turn_id = turn('c');
    let mut prior_turn_claim = claim(ArtifactExpectation::Required);
    assert!(prior_turn_claim
        .discover_controls(vec![wrong_turn], &bottom_proof())
        .is_err());
}

fn claim(expectation: ArtifactExpectation) -> ArtifactClaim {
    ArtifactClaim::establish(claim_id(), session_id().to_string(), turn('b'), expectation).unwrap()
}

fn baseline() -> ArtifactBaseline {
    ArtifactBaseline {
        directory: format!("artifacts/r-request/{}", claim_id()),
        entries: Vec::new(),
        captured_at_ms: 1_000,
        baseline_sha256: h256(b"baseline"),
    }
}

fn control() -> ArtifactControl {
    ArtifactControl {
        control_id: format!("control_{}", "1".repeat(64)),
        role: "button".to_string(),
        visible_text_hash: h256(b"download"),
        dom_path_hash: h256(b"dom"),
        bounding_box_hash: h256(b"box"),
        current_turn_id: turn('b'),
        visible: true,
        disabled: false,
    }
}

fn bottom_proof() -> BottomProof {
    BottomProof {
        at_bottom: true,
        method: "dom_terminal_anchor".to_string(),
        captured_at_ms: 1_000,
        evidence_refs: vec![evidence("requests/r/operations/a/bottom.json")],
    }
}

fn zero_proof() -> ZeroControlProof {
    ZeroControlProof {
        artifact_claim_id: claim_id(),
        terminal_assistant_turn_id: turn('b'),
        bottom_proof: bottom_proof(),
        control_count: 0,
        evidence_refs: vec![evidence("requests/r/operations/a/zero.json")],
        captured_at_ms: 1_001,
    }
}

fn download_receipt(bytes: &[u8]) -> PlaywrightDownloadReceipt {
    let echo = session_echo();
    let download_event_id = format!("download_{}", "3".repeat(64));
    let control = control();
    let artifact_id =
        derive_artifact_id(&claim_id(), &control.control_id, &download_event_id).unwrap();
    let path = artifact_host_saved_rel_path("r-request", &claim_id(), &artifact_id).unwrap();
    PlaywrightDownloadReceipt {
        artifact_claim_id: claim_id(),
        artifact_id,
        browser_context_id: echo.page_binding.browser_context_id,
        clicked_at_ms: 2_000,
        control,
        conversation_url: echo.conversation_url,
        download_event_id,
        host_saved_rel_path: path.clone(),
        listener_armed_at_ms: 1_999,
        media_type: mime_from_path(&path).to_string(),
        page_incarnation_id: echo.page_binding.page_incarnation_id,
        received_at_ms: 2_001,
        session_id: echo.session_id,
        sha256: h256(bytes),
        size_bytes: bytes.len() as u64,
        slot_id: echo.page_binding.slot_id,
        target_id: echo.page_binding.target_id,
        terminal_assistant_turn_id: turn('b'),
    }
}

fn session_echo() -> SessionEcho {
    SessionEcho {
        page_binding: PageBindingEcho {
            binding_id: format!("binding_{}", "4".repeat(64)),
            binding_generation: 1,
            slot_id: "slot-01".to_string(),
            cohort: "cohort-a".to_string(),
            lease_id: format!("lease_{}", "5".repeat(64)),
            lease_generation: 1,
            runtime_owner_id: format!("owner_{}", "6".repeat(64)),
            runtime_owner_generation: 1,
            runtime_incarnation_id: format!("runtime_{}", "7".repeat(64)),
            browser_context_id: format!("ctx_{}", "8".repeat(64)),
            target_id: format!("target_{}", "9".repeat(64)),
            page_incarnation_id: format!("page_{}", "a".repeat(64)),
            root_binding_hash: h256(b"root"),
            dom_mutation_generation: 1,
        },
        session_id: session_id().to_string(),
        conversation_url: format!("https://chatgpt.com/c/{}", session_id()),
        request_id: Some("request-1".to_string()),
        run_id: Some("run-1".to_string()),
        session_binding_id: format!("binding_{}", "b".repeat(64)),
        page_binding_generation: 1,
        visible_user_turn_id: Some(turn('a')),
        visible_assistant_turn_id: Some(turn('b')),
        active_turn: false,
        terminal_answer_sha256: Some(h256(b"answer")),
    }
}

fn evidence(path: &str) -> EvidenceRef {
    EvidenceRef {
        path: path.to_string(),
        sha256: h256(path.as_bytes()),
        size_bytes: 1,
        media_type: EvidenceMediaType::Json,
    }
}

fn claim_id() -> String {
    format!("artifact_claim_{}", "1".repeat(64))
}

fn session_id() -> &'static str {
    "6a623c19-bb00-83ee-bb64-691d8bff937b"
}

fn turn(value: char) -> String {
    format!("turn_{}", value.to_string().repeat(64))
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gpt-webai-lifecycle-artifact-r13-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
