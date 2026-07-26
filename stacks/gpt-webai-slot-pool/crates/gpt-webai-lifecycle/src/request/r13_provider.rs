use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::contracts::browser::{
    Effort, EffortProof, EvidenceRef, FailureProof, Model, ModelProof, PageBindingEcho,
    RootBindingCandidate,
};
use crate::contracts::provider::{
    ProviderEvidencePaths, ProviderIdentity, ProviderOperation, ProviderRequest, ReceiptRelPaths,
    REQUEST_SCHEMA,
};
use crate::provider_client::{
    run_r13_provider_invocation, R13ProviderInvocation, R13ProviderInvocationError,
};
use crate::provider_runner::R13ProviderCommand;
use crate::send_reconcile::SendReceipt;
use crate::uploads::{AttachmentSet, ChipProof, PromptInput, UploadProof};

#[derive(Clone, Copy, Debug)]
pub struct FreshProviderLimits {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct FreshProviderResult<T> {
    pub data: T,
    pub ok: bool,
    pub provider_reason: Option<String>,
    pub receipt: EvidenceRef,
    pub receipt_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureData {
    pub root_binding_candidate: Option<RootBindingCandidate>,
    pub failure_proof: Option<FailureProof>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelData {
    pub model_proof: Option<ModelProof>,
    pub effort_proof: Option<EffortProof>,
    pub failure_proof: Option<FailureProof>,
    pub observed_page_binding: Option<PageBindingEcho>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UploadData {
    pub upload_proof: Option<UploadProof>,
    pub failure_reason: Option<String>,
    pub observed_page_binding: Option<PageBindingEcho>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClearData {
    pub clear_attempt_id: String,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub attempted_chip_keys: Vec<String>,
    pub cleared_chips: Vec<Value>,
    pub observed_page_binding: Option<PageBindingEcho>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SendData {
    pub pre_click_receipt: SendReceipt,
    pub terminal_send_receipt: Option<SendReceipt>,
    pub observed_page_binding: Option<PageBindingEcho>,
}

#[derive(Debug, Error)]
pub enum FreshProviderError {
    #[error("R13 provider invocation failed: {0}")]
    Invocation(#[from] R13ProviderInvocationError),
    #[error("R13 provider response JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("R13 provider receipt path is invalid")]
    ReceiptPath,
}

pub fn capture_request(
    identity: ProviderIdentity,
    model: Model,
    effort: Effort,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::CaptureRoot,
        45_000,
        json!({
            "requestedModel": model,
            "requestedEffort": effort,
            "rediscoveryAttempt": 0
        }),
    )
}

pub fn model_request(
    identity: ProviderIdentity,
    page: &PageBindingEcho,
    model: Model,
    effort: Effort,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::EnsureModel,
        60_000,
        json!({
            "pageBinding":page,"requestedModel":model,"requestedEffort":effort,
            "pickerOpenBudget":1,"stabilizationMs":500
        }),
    )
}

pub fn upload_request(
    identity: ProviderIdentity,
    page: &PageBindingEcho,
    set: &AttachmentSet,
    upload_attempt_id: &str,
    retry_index: u8,
) -> ProviderRequest {
    let records = set
        .records
        .iter()
        .map(|record| {
            json!({
                "ordinal":record.ordinal,"containerRelPath":record.container_rel_path,
                "sourceSha256":record.source_sha256,"sizeBytes":record.size_bytes,
                "mediaType":record.media_type
            })
        })
        .collect::<Vec<_>>();
    request(
        identity,
        ProviderOperation::UploadOnly,
        180_000,
        json!({
            "pageBinding":page,
            "attachmentSet":{"count":set.count,"records":records,"setSha256":set.set_sha256},
            "uploadAttemptId":upload_attempt_id,"retryIndex":retry_index
        }),
    )
}

pub fn clear_request(
    identity: ProviderIdentity,
    page: &PageBindingEcho,
    upload_attempt_id: &str,
    clear_attempt_id: &str,
    stale_chips: &[ChipProof],
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::ClearUpload,
        45_000,
        json!({
            "pageBinding":page,"uploadAttemptId":upload_attempt_id,
            "clearAttemptId":clear_attempt_id,"staleChips":stale_chips
        }),
    )
}

pub fn send_click_request(
    identity: ProviderIdentity,
    page: &PageBindingEcho,
    send_attempt_id: &str,
    upload_proof: &UploadProof,
    prompt: &PromptInput,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::SendClick,
        180_000,
        json!({
            "pageBinding":page,"sendAttemptId":send_attempt_id,
            "uploadProof":upload_proof,"promptInput":prompt,"clickBudget":1
        }),
    )
}

pub fn send_reconcile_request(
    identity: ProviderIdentity,
    page: &PageBindingEcho,
    send_attempt_id: &str,
    pre_click_receipt: &SendReceipt,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::SendReconcile,
        90_000,
        json!({
            "pageBinding":page,"sendAttemptId":send_attempt_id,
            "preClickReceipt":pre_click_receipt
        }),
    )
}

pub fn invoke<T: DeserializeOwned>(
    state_root: &Path,
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    limits: FreshProviderLimits,
) -> Result<FreshProviderResult<T>, FreshProviderError> {
    let result = run_r13_provider_invocation(&R13ProviderInvocation {
        command,
        request,
        state_root,
        timeout: limits.timeout,
        max_stdout_bytes: limits.max_stdout_bytes,
        max_stderr_bytes: limits.max_stderr_bytes,
    })?;
    let response = result.response;
    let data = serde_json::from_value(response.operation_data.clone())?;
    Ok(FreshProviderResult {
        data,
        ok: response.ok,
        provider_reason: response.provider_reason,
        receipt: state_relative_receipt(state_root, command, response.receipt)?,
        receipt_ids: result.receipt_ids,
    })
}

pub fn identity(
    cohort: &str,
    operation_id: &str,
    request_id: &str,
    run_id: &str,
    slot_id: &str,
) -> ProviderIdentity {
    ProviderIdentity {
        cohort: Some(cohort.to_string()),
        operation_id: operation_id.to_string(),
        request_id: Some(request_id.to_string()),
        run_id: Some(run_id.to_string()),
        session_id: None,
        slot_id: slot_id.to_string(),
    }
}

fn request(
    identity: ProviderIdentity,
    operation: ProviderOperation,
    deadline_ms: u64,
    operation_data: Value,
) -> ProviderRequest {
    let send_click = operation == ProviderOperation::SendClick;
    let send_reconcile = operation == ProviderOperation::SendReconcile;
    ProviderRequest {
        deadline_ms,
        evidence: ProviderEvidencePaths {
            cdp_rel_path: "cdp.sanitized.json".to_string(),
            dom_rel_path: "dom.sanitized.json".to_string(),
            receipt_rel_paths: ReceiptRelPaths {
                primary: "provider-receipt.json".to_string(),
                pre_click: (send_click || send_reconcile)
                    .then(|| "send.pre-click.receipt.json".to_string()),
                post_click: send_click.then(|| "send.post-click.receipt.json".to_string()),
                reconcile: send_reconcile.then(|| "send.reconcile.receipt.json".to_string()),
            },
            screenshot_rel_path: "screenshot.privacy-crop.png".to_string(),
        },
        identity,
        operation,
        operation_data,
        schema: REQUEST_SCHEMA.to_string(),
    }
}

fn state_relative_receipt(
    state_root: &Path,
    command: &R13ProviderCommand,
    mut receipt: EvidenceRef,
) -> Result<EvidenceRef, FreshProviderError> {
    receipt.path = command
        .paths
        .operation_host_dir
        .join(&receipt.path)
        .strip_prefix(state_root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or(FreshProviderError::ReceiptPath)?;
    receipt
        .validate()
        .map_err(|_| FreshProviderError::ReceiptPath)?;
    Ok(receipt)
}
