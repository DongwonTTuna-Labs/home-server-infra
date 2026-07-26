use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::artifact_claims::completion::PlaywrightDownloadReceipt;
use crate::artifact_claims::{ArtifactControl, BottomProof, ZeroControlProof};
use crate::contracts::browser::{EvidenceRef, SessionEcho, SessionRebindExpectation};
use crate::contracts::health::HealthStatus;
use crate::contracts::provider::{
    ProviderEvidencePaths, ProviderIdentity, ProviderOperation, ProviderRequest, ReceiptRelPaths,
    REQUEST_SCHEMA,
};
use crate::provider_client::{
    run_r13_provider_invocation, R13ProviderInvocation, R13ProviderInvocationError,
    R13ProviderInvocationResult,
};
use crate::provider_runner::R13ProviderCommand;
use crate::session_rebind::hydration::{HydrationObservation, HydrationTrace};
use crate::session_rebind::{RebindProof, SessionRebindError, TerminalAnswerObservation};

#[derive(Clone, Copy, Debug)]
pub struct ProviderLimits {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct RebindInvocationResult {
    pub proof: Option<RebindProof>,
    pub failure_reason: Option<String>,
    pub receipt: EvidenceRef,
    pub receipt_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct StatusInvocationResult {
    pub ok: bool,
    pub composer_ready: bool,
    pub docker_status: String,
    pub health_status: HealthStatus,
    pub model_label: String,
    pub retry_after_ms: Option<u64>,
    pub failure_reason: Option<String>,
    pub receipt: EvidenceRef,
    pub receipt_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PollResponseData {
    pub expected: SessionEcho,
    pub observed_echo: Option<SessionEcho>,
    pub poll_state: String,
    pub answer_sha256: Option<String>,
    pub answer_size_bytes: Option<u64>,
    pub answer_rel_path: Option<String>,
    pub terminal_assistant_turn_id: Option<String>,
    pub bottom_proof: Option<BottomProof>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactDiscoverResponseData {
    pub controls: Vec<ArtifactControl>,
    pub bottom_proof: Option<BottomProof>,
    pub zero_control_proof: Option<ZeroControlProof>,
    pub failure_reason: Option<String>,
    pub observed_echo: Option<SessionEcho>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactClickResponseData {
    pub download_receipt: Option<PlaywrightDownloadReceipt>,
    pub failure_reason: Option<String>,
    pub observed_echo: Option<SessionEcho>,
}

#[derive(Clone, Debug)]
pub struct SessionProviderResult<T> {
    pub data: T,
    pub ok: bool,
    pub provider_reason: Option<String>,
    pub receipt: EvidenceRef,
    pub receipt_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum RebindProviderError {
    #[error("R13 provider invocation failed: {0}")]
    Invocation(#[from] R13ProviderInvocationError),
    #[error("R13 session-rebind response invalid: {0}")]
    Response(&'static str),
    #[error("R13 session-rebind proof invalid: {0}")]
    Proof(#[from] SessionRebindError),
    #[error("R13 session-rebind response json invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RebindResponseData {
    expectation: SessionRebindExpectation,
    observed_echo: Option<SessionEcho>,
    page_binding_generation: Option<u16>,
    hydration_observations: Vec<HydrationObservation>,
    terminal_answer: Option<TerminalAnswerObservation>,
    failure_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StatusResponseData {
    composer_ready: bool,
    docker_status: String,
    health_status: HealthStatus,
    model_label: String,
    retry_after_ms: Option<u64>,
}

pub fn build_status_request(
    identity: ProviderIdentity,
    slot_id: &str,
    probe_attempt: u8,
) -> ProviderRequest {
    ProviderRequest {
        deadline_ms: 15_000,
        evidence: ProviderEvidencePaths {
            cdp_rel_path: "cdp.sanitized.json".to_string(),
            dom_rel_path: "dom.sanitized.json".to_string(),
            receipt_rel_paths: ReceiptRelPaths {
                primary: "provider-receipt.json".to_string(),
                pre_click: None,
                post_click: None,
                reconcile: None,
            },
            screenshot_rel_path: "screenshot.privacy-crop.png".to_string(),
        },
        identity,
        operation: ProviderOperation::Status,
        operation_data: json!({
            "expectedSlotId": slot_id,
            "probeAttempt": probe_attempt
        }),
        schema: REQUEST_SCHEMA.to_string(),
    }
}

pub fn invoke_status(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<StatusInvocationResult, RebindProviderError> {
    let result = run_r13_provider_invocation(&R13ProviderInvocation {
        command,
        request,
        state_root,
        timeout: limits.timeout,
        max_stdout_bytes: limits.max_stdout_bytes,
        max_stderr_bytes: limits.max_stderr_bytes,
    })?;
    let response = result.response;
    let data: StatusResponseData = serde_json::from_value(response.operation_data.clone())?;
    Ok(StatusInvocationResult {
        ok: response.ok,
        composer_ready: data.composer_ready,
        docker_status: data.docker_status,
        health_status: data.health_status,
        model_label: data.model_label,
        retry_after_ms: data.retry_after_ms,
        failure_reason: response.provider_reason,
        receipt: state_relative_receipt(state_root, command, response.receipt)?,
        receipt_ids: result.receipt_ids,
    })
}

pub fn build_rebind_request(
    expectation: &SessionRebindExpectation,
    operation_kind: &str,
    operation_id: &str,
) -> ProviderRequest {
    ProviderRequest {
        deadline_ms: 170_000,
        evidence: ProviderEvidencePaths {
            cdp_rel_path: "cdp.sanitized.json".to_string(),
            dom_rel_path: "dom.sanitized.json".to_string(),
            receipt_rel_paths: ReceiptRelPaths {
                primary: "provider-receipt.json".to_string(),
                pre_click: None,
                post_click: None,
                reconcile: None,
            },
            screenshot_rel_path: "screenshot.privacy-crop.png".to_string(),
        },
        identity: ProviderIdentity {
            cohort: Some(expectation.cohort.clone()),
            operation_id: operation_id.to_string(),
            request_id: expectation.request_id.clone(),
            run_id: expectation.run_id.clone(),
            session_id: Some(expectation.session_id.clone()),
            slot_id: expectation.slot_id.clone(),
        },
        operation: ProviderOperation::SessionRebind,
        operation_data: json!({
            "operationKind": operation_kind,
            "expectation": expectation,
            "navigationAttemptLimit": 2,
            "hydrationDeadlineMs": 90_000
        }),
        schema: REQUEST_SCHEMA.to_string(),
    }
}

pub fn build_poll_request(
    identity: ProviderIdentity,
    expected: &SessionEcho,
    poll_attempt_id: &str,
    poll_timeout_seconds: u64,
    artifact_expectation: &str,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::Poll,
        170_000 + poll_timeout_seconds.saturating_mul(1_000),
        json!({
            "expected": expected,
            "pollAttemptId": poll_attempt_id,
            "pollTimeoutSeconds": poll_timeout_seconds,
            "artifactExpectation": artifact_expectation
        }),
    )
}

pub fn build_artifact_discover_request(
    identity: ProviderIdentity,
    expected: &SessionEcho,
    artifact_claim_id: &str,
    terminal_assistant_turn_id: &str,
    expectation: &str,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::ArtifactDiscover,
        120_000,
        json!({
            "expected": expected,
            "artifactClaimId": artifact_claim_id,
            "terminalAssistantTurnId": terminal_assistant_turn_id,
            "expectation": expectation
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_artifact_click_request(
    identity: ProviderIdentity,
    expected: &SessionEcho,
    artifact_claim_id: &str,
    terminal_assistant_turn_id: &str,
    control: &ArtifactControl,
    baseline: &crate::artifact_claims::baseline::ArtifactBaseline,
    control_index: u8,
    host_save_directory: &str,
) -> ProviderRequest {
    request(
        identity,
        ProviderOperation::ArtifactClickSave,
        120_000,
        json!({
            "expected": expected,
            "artifactClaimId": artifact_claim_id,
            "terminalAssistantTurnId": terminal_assistant_turn_id,
            "control": control,
            "baseline": baseline,
            "controlIndex": control_index,
            "hostSaveDirectory": host_save_directory
        }),
    )
}

pub fn invoke_poll(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<SessionProviderResult<PollResponseData>, RebindProviderError> {
    invoke_data(command, request, state_root, limits)
}

pub fn invoke_artifact_discover(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<SessionProviderResult<ArtifactDiscoverResponseData>, RebindProviderError> {
    invoke_data(command, request, state_root, limits)
}

pub fn invoke_artifact_click(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<SessionProviderResult<ArtifactClickResponseData>, RebindProviderError> {
    invoke_data(command, request, state_root, limits)
}

pub fn invoke_rebind(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<RebindInvocationResult, RebindProviderError> {
    let result = run_r13_provider_invocation(&R13ProviderInvocation {
        command,
        request,
        state_root,
        timeout: limits.timeout,
        max_stdout_bytes: limits.max_stdout_bytes,
        max_stderr_bytes: limits.max_stderr_bytes,
    })?;
    parse_rebind_result(result, request, state_root, command)
}

fn parse_rebind_result(
    result: R13ProviderInvocationResult,
    request: &ProviderRequest,
    state_root: &Path,
    command: &R13ProviderCommand,
) -> Result<RebindInvocationResult, RebindProviderError> {
    let response = result.response;
    let data: RebindResponseData = serde_json::from_value(response.operation_data.clone())?;
    if data.expectation
        != serde_json::from_value::<SessionRebindExpectation>(
            request.operation_data["expectation"].clone(),
        )?
    {
        return Err(RebindProviderError::Response("expectation echo"));
    }
    let proof = if response.ok {
        let proof = RebindProof {
            expectation: data.expectation,
            observed_echo: data
                .observed_echo
                .ok_or(RebindProviderError::Response("observedEcho"))?,
            page_binding_generation: data
                .page_binding_generation
                .ok_or(RebindProviderError::Response("pageBindingGeneration"))?,
            hydration: HydrationTrace {
                observations: data.hydration_observations,
            },
            terminal_answer: data.terminal_answer,
        };
        let expected: SessionRebindExpectation =
            serde_json::from_value(request.operation_data["expectation"].clone())?;
        proof.validate(&expected)?;
        if data.failure_reason.is_some() {
            return Err(RebindProviderError::Response("failureReason"));
        }
        Some(proof)
    } else {
        if data.failure_reason.as_deref() != response.provider_reason.as_deref() {
            return Err(RebindProviderError::Response("failureReason"));
        }
        None
    };
    Ok(RebindInvocationResult {
        proof,
        failure_reason: response.provider_reason,
        receipt: state_relative_receipt(state_root, command, response.receipt)?,
        receipt_ids: result.receipt_ids,
    })
}

fn state_relative_receipt(
    state_root: &Path,
    command: &R13ProviderCommand,
    mut receipt: EvidenceRef,
) -> Result<EvidenceRef, RebindProviderError> {
    let absolute = command.paths.operation_host_dir.join(&receipt.path);
    let relative = absolute
        .strip_prefix(state_root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or(RebindProviderError::Response("receipt path"))?;
    receipt.path = relative;
    receipt
        .validate()
        .map_err(|_| RebindProviderError::Response("receipt path"))?;
    Ok(receipt)
}

fn request(
    identity: ProviderIdentity,
    operation: ProviderOperation,
    deadline_ms: u64,
    operation_data: serde_json::Value,
) -> ProviderRequest {
    ProviderRequest {
        deadline_ms,
        evidence: ProviderEvidencePaths {
            cdp_rel_path: "cdp.sanitized.json".to_string(),
            dom_rel_path: "dom.sanitized.json".to_string(),
            receipt_rel_paths: ReceiptRelPaths {
                primary: "provider-receipt.json".to_string(),
                pre_click: None,
                post_click: None,
                reconcile: None,
            },
            screenshot_rel_path: "screenshot.privacy-crop.png".to_string(),
        },
        identity,
        operation,
        operation_data,
        schema: REQUEST_SCHEMA.to_string(),
    }
}

fn invoke_data<T: DeserializeOwned>(
    command: &R13ProviderCommand,
    request: &ProviderRequest,
    state_root: &Path,
    limits: ProviderLimits,
) -> Result<SessionProviderResult<T>, RebindProviderError> {
    let result = run_r13_provider_invocation(&R13ProviderInvocation {
        command,
        request,
        state_root,
        timeout: limits.timeout,
        max_stdout_bytes: limits.max_stdout_bytes,
        max_stderr_bytes: limits.max_stderr_bytes,
    })?;
    let response = result.response;
    Ok(SessionProviderResult {
        data: serde_json::from_value(response.operation_data.clone())?,
        ok: response.ok,
        provider_reason: response.provider_reason,
        receipt: state_relative_receipt(state_root, command, response.receipt)?,
        receipt_ids: result.receipt_ids,
    })
}
