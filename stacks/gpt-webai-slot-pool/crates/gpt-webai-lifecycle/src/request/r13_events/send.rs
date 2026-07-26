use serde_json::json;
use thiserror::Error;

use crate::contracts::browser::{
    Effort, EffortProof, EvidenceRef, FailureProof, Model, ModelProof, PageBindingEcho,
    RootBindingCandidate,
};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::{derive_session_binding_id, IdError};
use crate::send_reconcile::{validate_receipt_pair, SendReceipt, SendReconcileError, TurnStart};
use crate::session_ops::journal::{NewEvent, SessionJournal, SessionJournalError};

use super::event_time;
use crate::request::r13_assets::MaterializedFile;

#[derive(Debug, Error)]
pub enum SendEventError {
    #[error("R13 send event journal failed: {0}")]
    Journal(#[from] SessionJournalError),
    #[error("R13 send event identifier failed: {0}")]
    Id(#[from] IdError),
    #[error("R13 send event receipt failed: {0}")]
    Receipt(#[from] SendReconcileError),
    #[error("R13 send event JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("R13 send event contract failed: {0}")]
    Contract(&'static str),
}

pub struct BindingEvents {
    pub turn: EventEnvelope,
    pub binding: EventEnvelope,
    pub session_binding_id: String,
    pub turn_start: TurnStart,
}

pub fn append_root_started(
    journal: &mut SessionJournal,
    staged: &EventEnvelope,
    health: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    slot_id: &str,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        staged,
        EventType::RootCaptureStarted,
        request_id,
        json!({
            "requestId":request_id,"captureOperationId":operation_id,"slotId":slot_id,
            "startedAtMs":event_time(Some(staged))
        }),
        vec![health.event_id.clone()],
    )
}

pub fn append_root_observed(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    root: &RootBindingCandidate,
    page: &PageBindingEcho,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::RootCaptureObserved,
        request_id,
        json!({
            "requestId":request_id,"captureOperationId":operation_id,"rootBindingCandidate":root,
            "bindingId":page.binding_id,"bindingGeneration":1,"pageBinding":page,
            "observedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

pub fn append_root_failed(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::RootCaptureFailed,
        request_id,
        json!({
            "requestId":request_id,"captureOperationId":operation_id,
            "reason":reason,"providerReceipt":provider_receipt,
            "failedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

pub fn append_model_started(
    journal: &mut SessionJournal,
    root: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    model: &Model,
    effort: &Effort,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        root,
        EventType::ModelSelectionStarted,
        request_id,
        json!({
            "requestId":request_id,"modelOperationId":operation_id,"requestedModel":model,
            "requestedEffort":effort,"startedAtMs":event_time(Some(root))
        }),
        vec![root.event_id.clone()],
    )
}

pub fn append_model_verified(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    model: &ModelProof,
    effort: &EffortProof,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::ModelSelectionVerified,
        request_id,
        json!({
            "requestId":request_id,"modelOperationId":operation_id,"modelProof":model,
            "effortProof":effort,"failureProof":null,"verifiedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

pub fn append_model_failed(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    operation_id: &str,
    reason: &str,
    failure_proof: Option<&FailureProof>,
    provider_receipt: Option<&EvidenceRef>,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::ModelSelectionFailed,
        request_id,
        json!({
            "requestId":request_id,"modelOperationId":operation_id,
            "modelProof":null,"effortProof":null,"failureProof":failure_proof,
            "reason":reason,"providerReceipt":provider_receipt,
            "failedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

pub fn append_materialized(
    journal: &mut SessionJournal,
    model: &EventEnvelope,
    request_id: &str,
    slot_id: &str,
    run_id: &str,
    set_sha256: &str,
    files: &[MaterializedFile],
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        model,
        EventType::SlotAttachmentsMaterialized,
        request_id,
        json!({
            "requestId":request_id,"slotId":slot_id,"attachmentSetSha256":set_sha256,
            "containerMountRoot":run_id,"materializedFiles":files,
            "materializedAtMs":event_time(Some(model))
        }),
        vec![model.event_id.clone()],
    )
}

pub fn append_upload_started(
    journal: &mut SessionJournal,
    predecessor: &EventEnvelope,
    request_id: &str,
    attempt_id: &str,
    retry_index: u8,
    set_sha256: &str,
    binding_hash: &str,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        predecessor,
        EventType::UploadStarted,
        request_id,
        json!({
            "requestId":request_id,"uploadAttemptId":attempt_id,"retryIndex":retry_index,
            "expectedSetSha256":set_sha256,"expectedBindingHash":binding_hash,
            "startedAtMs":event_time(Some(predecessor))
        }),
        vec![predecessor.event_id.clone()],
    )
}

pub fn append_upload_completed(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    proof: &crate::uploads::UploadProof,
    receipt: &EvidenceRef,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::UploadCompleted,
        request_id,
        json!({
            "requestId":request_id,"uploadAttemptId":proof.upload_attempt_id,
            "retryIndex":proof.retry_index,"uploadProof":proof,"providerReceipt":receipt,
            "completedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

pub fn append_upload_failed(
    journal: &mut SessionJournal,
    started: &EventEnvelope,
    request_id: &str,
    attempt_id: &str,
    retry_index: u8,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        started,
        EventType::UploadFailed,
        request_id,
        json!({
            "requestId":request_id,"uploadAttemptId":attempt_id,
            "retryIndex":retry_index,"reason":reason,
            "providerReceipt":provider_receipt,"failedAtMs":event_time(Some(started))
        }),
        vec![started.event_id.clone()],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append_send_armed(
    journal: &mut SessionJournal,
    upload: &EventEnvelope,
    request_id: &str,
    send_attempt_id: &str,
    upload_attempt_id: &str,
    page: &PageBindingEcho,
    prompt_sha256: &str,
    receipt_paths: [&str; 3],
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        upload,
        EventType::SendClickArmed,
        request_id,
        json!({
            "requestId":request_id,"sendAttemptId":send_attempt_id,
            "uploadAttemptId":upload_attempt_id,"expectedBindingHash":page.root_binding_hash,
            "promptSha256":prompt_sha256,"preClickReceiptPath":receipt_paths[0],
            "postClickReceiptPath":receipt_paths[1],"reconcileReceiptPath":receipt_paths[2],
            "clickBudget":1,"armedAtMs":event_time(Some(upload))
        }),
        vec![upload.event_id.clone()],
    )
}

pub fn append_send_clicked(
    journal: &mut SessionJournal,
    armed: &EventEnvelope,
    request_id: &str,
    pre: &SendReceipt,
    post: &SendReceipt,
) -> Result<EventEnvelope, SendEventError> {
    let turn = validate_receipt_pair(pre, post, &pre.page_binding)?;
    append_request(
        journal,
        armed,
        EventType::SendClicked,
        request_id,
        json!({
            "requestId":request_id,"sendAttemptId":pre.send_attempt_id,"preClickReceipt":pre,
            "postClickReceipt":post,"physicalClickCount":turn.physical_click_count,
            "clickedAtMs":post.captured_at_ms
        }),
        vec![armed.event_id.clone()],
    )
}

pub fn append_send_reconciled(
    journal: &mut SessionJournal,
    armed: &EventEnvelope,
    request_id: &str,
    pre: &SendReceipt,
    reconciled: &SendReceipt,
) -> Result<EventEnvelope, SendEventError> {
    let turn = validate_receipt_pair(pre, reconciled, &pre.page_binding)?;
    append_request(
        journal,
        armed,
        EventType::SendReconciled,
        request_id,
        json!({
            "requestId":request_id,"sendAttemptId":pre.send_attempt_id,"preClickReceipt":pre,
            "reconciledReceipt":reconciled,"physicalClickCount":turn.physical_click_count,
            "reconciledAtMs":reconciled.captured_at_ms
        }),
        vec![armed.event_id.clone()],
    )
}

pub fn append_send_uncertain(
    journal: &mut SessionJournal,
    armed: &EventEnvelope,
    request_id: &str,
    send_attempt_id: &str,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        armed,
        EventType::SendUncertain,
        request_id,
        json!({
            "requestId":request_id,"sendAttemptId":send_attempt_id,
            "reason":"send.turn_not_proven","blockedAtMs":event_time(Some(armed))
        }),
        vec![armed.event_id.clone()],
    )
}

pub fn append_send_failed(
    journal: &mut SessionJournal,
    armed: &EventEnvelope,
    request_id: &str,
    send_attempt_id: &str,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        armed,
        EventType::SendFailed,
        request_id,
        json!({
            "requestId":request_id,"sendAttemptId":send_attempt_id,
            "reason":reason,"providerReceipt":provider_receipt,
            "failedAtMs":event_time(Some(armed))
        }),
        vec![armed.event_id.clone()],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append_binding(
    journal: &mut SessionJournal,
    clicked: &EventEnvelope,
    root: &EventEnvelope,
    request_id: &str,
    slot_id: &str,
    cohort: &str,
    page: &PageBindingEcho,
    pre: &SendReceipt,
    terminal: &SendReceipt,
) -> Result<BindingEvents, SendEventError> {
    let start = validate_receipt_pair(pre, terminal, page)?;
    let at = event_time(Some(clicked));
    let turn = append_request(
        journal,
        clicked,
        EventType::TurnStartConfirmed,
        request_id,
        json!({
            "requestId":request_id,"sessionId":start.session_id,"conversationUrl":start.conversation_url,
            "userTurnId":start.user_turn_id,"assistantTurnId":start.assistant_turn_id,
            "confirmedAtMs":at
        }),
        vec![clicked.event_id.clone()],
    )?;
    let session_binding_id = derive_session_binding_id(&start.session_id, slot_id, cohort)?;
    let binding = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: start.session_id.clone(),
        event_type: EventType::SessionBindingEstablished,
        payload: json!({"sessionId":start.session_id,"sessionBindingId":session_binding_id,
            "conversationUrl":start.conversation_url,"slotId":slot_id,"cohort":cohort,
            "pageBindingId":page.binding_id,"pageBindingGeneration":1,"targetId":page.target_id,
            "pageIncarnationId":page.page_incarnation_id,"runtimeOwnerId":page.runtime_owner_id,
            "establishedAtMs":event_time(Some(&turn))}),
        predecessor_event_id: None,
        source_event_ids: vec![turn.event_id.clone(), root.event_id.clone()],
        created_at_ms: event_time(Some(&turn)),
    })?;
    Ok(BindingEvents {
        turn,
        binding,
        session_binding_id,
        turn_start: start,
    })
}

pub fn append_running(
    journal: &mut SessionJournal,
    turn: &EventEnvelope,
    binding: &EventEnvelope,
    request_id: &str,
    session_id: &str,
    session_binding_id: &str,
) -> Result<EventEnvelope, SendEventError> {
    append_request(
        journal,
        turn,
        EventType::RunningProjected,
        request_id,
        json!({
            "requestId":request_id,"sessionId":session_id,
            "sessionBindingId":session_binding_id,"activeTurn":true,
            "projectedAtMs":event_time(Some(turn))
        }),
        vec![binding.event_id.clone()],
    )
}

fn append_request(
    journal: &mut SessionJournal,
    predecessor: &EventEnvelope,
    event_type: EventType,
    request_id: &str,
    payload: serde_json::Value,
    source_event_ids: Vec<String>,
) -> Result<EventEnvelope, SendEventError> {
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.to_string(),
        event_type,
        payload,
        predecessor_event_id: Some(predecessor.event_id.clone()),
        source_event_ids,
        created_at_ms: event_time(Some(predecessor)),
    })?)
}
