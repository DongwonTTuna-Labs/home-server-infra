use gpt_webai_lifecycle::contracts::browser::{EvidenceMediaType, EvidenceRef, PageBindingEcho};
use gpt_webai_lifecycle::contracts::ids::{derive_page_binding_id, h256};
use gpt_webai_lifecycle::send_reconcile::state::{ArmedSend, SendState};
use gpt_webai_lifecycle::send_reconcile::{
    validate_receipt_pair, SendReceipt, SendReceiptKind, SendReconcileError,
};

#[test]
fn fresh_path_allows_exactly_one_physical_click() {
    let armed = armed();
    let pre = receipt(SendReceiptKind::PreClick, 0);
    let post = receipt(SendReceiptKind::PostClick, 1);
    let state = SendState::freshly_armed(armed.clone())
        .unwrap()
        .begin_physical_click()
        .unwrap()
        .accept_terminal(&pre, &post, &armed.page_binding)
        .unwrap();
    assert_eq!(state.turn_start().unwrap().physical_click_count, 1);
    assert!(matches!(
        state.begin_physical_click(),
        Err(SendReconcileError::IllegalTransition)
    ));
}

#[test]
fn restarted_armed_state_can_only_reconcile_without_click() {
    let armed = armed();
    let state = SendState::recover_armed(armed.clone()).unwrap();
    assert!(matches!(
        state.clone().begin_physical_click(),
        Err(SendReconcileError::IllegalTransition)
    ));
    let pre = receipt(SendReceiptKind::PreClick, 0);
    let reconciled = receipt(SendReceiptKind::ReconciledTurnStart, 0);
    let state = state
        .begin_reconcile()
        .unwrap()
        .accept_terminal(&pre, &reconciled, &armed.page_binding)
        .unwrap();
    assert_eq!(state.turn_start().unwrap().physical_click_count, 0);
}

#[test]
fn recovery_without_turn_proof_is_uncertain_and_never_reclicks() {
    let state = SendState::recover_armed(armed())
        .unwrap()
        .begin_reconcile()
        .unwrap()
        .mark_uncertain()
        .unwrap();
    assert!(state.turn_start().is_none());
    assert!(matches!(
        state.begin_physical_click(),
        Err(SendReconcileError::IllegalTransition)
    ));
}

#[test]
fn rejects_placeholder_root_or_mismatched_receipts() {
    let pre = receipt(SendReceiptKind::PreClick, 0);
    let mut post = receipt(SendReceiptKind::PostClick, 1);
    post.session_id = Some("WEB:placeholder".to_string());
    post.conversation_url = Some("https://chatgpt.com/".to_string());
    assert!(validate_receipt_pair(&pre, &post, &binding()).is_err());

    let mut post = receipt(SendReceiptKind::PostClick, 1);
    post.prompt_sha256 = h256(b"other");
    assert!(validate_receipt_pair(&pre, &post, &binding()).is_err());
}

#[test]
fn observed_page_binding_must_equal_the_armed_binding() {
    let armed = armed();
    let pre = receipt(SendReceiptKind::PreClick, 0);
    let post = receipt(SendReceiptKind::PostClick, 1);
    let mut observed = armed.page_binding.clone();
    observed.dom_mutation_generation += 1;
    let result = SendState::freshly_armed(armed)
        .unwrap()
        .begin_physical_click()
        .unwrap()
        .accept_terminal(&pre, &post, &observed);
    assert!(matches!(
        result,
        Err(SendReconcileError::Invalid("terminal response binding"))
    ));
}

fn armed() -> ArmedSend {
    ArmedSend {
        send_attempt_id: "send-1".to_string(),
        page_binding: binding(),
        prompt_sha256: h256(b"prompt"),
    }
}

fn receipt(kind: SendReceiptKind, click_count: u8) -> SendReceipt {
    let terminal = kind != SendReceiptKind::PreClick;
    SendReceipt {
        kind,
        send_attempt_id: "send-1".to_string(),
        page_binding: binding(),
        prompt_sha256: h256(b"prompt"),
        physical_click_count: click_count,
        user_turn_id: terminal.then(|| format!("turn_{}", "1".repeat(64))),
        assistant_turn_id: terminal.then(|| format!("turn_{}", "2".repeat(64))),
        session_id: terminal.then(|| "6a623c19-bb00-83ee-bb64-691d8bff937b".to_string()),
        conversation_url: terminal
            .then(|| "https://chatgpt.com/c/6a623c19-bb00-83ee-bb64-691d8bff937b".to_string()),
        captured_at_ms: if terminal { 2_000 } else { 1_000 },
        evidence_refs: vec![EvidenceRef {
            path: format!("requests/r/operations/send/{kind:?}.json"),
            sha256: h256(format!("{kind:?}").as_bytes()),
            size_bytes: 1,
            media_type: EvidenceMediaType::Json,
        }],
    }
}

fn binding() -> PageBindingEcho {
    let page_incarnation_id = format!("page_{}", "7".repeat(64));
    let root_binding_hash = h256(b"root");
    PageBindingEcho {
        binding_id: derive_page_binding_id(&page_incarnation_id, &root_binding_hash).unwrap(),
        binding_generation: 1,
        slot_id: "slot-01".to_string(),
        cohort: "cohort-a".to_string(),
        lease_id: format!("lease_{}", "2".repeat(64)),
        lease_generation: 1,
        runtime_owner_id: format!("owner_{}", "3".repeat(64)),
        runtime_owner_generation: 1,
        runtime_incarnation_id: format!("runtime_{}", "4".repeat(64)),
        browser_context_id: format!("ctx_{}", "5".repeat(64)),
        target_id: format!("target_{}", "6".repeat(64)),
        page_incarnation_id,
        root_binding_hash,
        dom_mutation_generation: 1,
    }
}
