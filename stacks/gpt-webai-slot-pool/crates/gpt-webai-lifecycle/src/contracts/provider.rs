use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
    artifact_claims::{
        baseline::ArtifactBaseline, completion::PlaywrightDownloadReceipt, ArtifactControl,
        ArtifactExpectation, BottomProof, ZeroControlProof,
    },
    send_reconcile::{validate_receipt_pair, SendReceipt, SendReceiptKind},
    session_rebind::{
        hydration::{HydrationObservation, HydrationTrace},
        RebindProof, TerminalAnswerObservation, HYDRATION_DEADLINE_MS, NAVIGATION_ATTEMPT_LIMIT,
    },
    uploads::{ChipProof, PromptInput, UploadProof, MAX_ATTACHMENTS},
};

use super::{
    browser::{
        validate_model_tuple, Effort, EffortProof, EvidenceMediaType, EvidenceRef, FailureProof,
        Model, ModelProof, PageBindingEcho, RootBindingCandidate, SessionEcho,
        SessionRebindExpectation,
    },
    health::HealthStatus,
    ids::{
        validate_artifact_claim_id, validate_byte_count, validate_cohort, validate_h256,
        validate_operation_id, validate_request_id, validate_run_id, validate_safe_rel_path,
        validate_session_id, validate_slot_id, validate_turn_id, MAX_DURATION_MS,
    },
};

pub const REQUEST_SCHEMA: &str = "gpt-webai.provider.request.r13.v1";
pub const RESPONSE_SCHEMA: &str = "gpt-webai.provider.response.r13.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProviderOperation {
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "capture.root")]
    CaptureRoot,
    #[serde(rename = "ensure-model")]
    EnsureModel,
    #[serde(rename = "upload-only")]
    UploadOnly,
    #[serde(rename = "clear-upload")]
    ClearUpload,
    #[serde(rename = "send-click")]
    SendClick,
    #[serde(rename = "send-reconcile")]
    SendReconcile,
    #[serde(rename = "session-rebind")]
    SessionRebind,
    #[serde(rename = "poll")]
    Poll,
    #[serde(rename = "artifact-discover")]
    ArtifactDiscover,
    #[serde(rename = "artifact-click-save")]
    ArtifactClickSave,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderIdentity {
    pub cohort: Option<String>,
    pub operation_id: String,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub slot_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReceiptRelPaths {
    pub primary: String,
    pub pre_click: Option<String>,
    pub post_click: Option<String>,
    pub reconcile: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderEvidencePaths {
    pub cdp_rel_path: String,
    pub dom_rel_path: String,
    pub receipt_rel_paths: ReceiptRelPaths,
    pub screenshot_rel_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderAttachmentRecord {
    ordinal: u8,
    container_rel_path: String,
    source_sha256: String,
    size_bytes: u64,
    media_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderAttachmentSet {
    count: u8,
    records: Vec<ProviderAttachmentRecord>,
    set_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClearedChip {
    chip_stable_key: String,
    digest: Option<String>,
    cleared: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderRequest {
    pub deadline_ms: u64,
    pub evidence: ProviderEvidencePaths,
    pub identity: ProviderIdentity,
    pub operation: ProviderOperation,
    pub operation_data: Value,
    pub schema: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderResponse {
    pub identity: ProviderIdentity,
    pub ok: bool,
    pub operation: ProviderOperation,
    pub operation_data: Value,
    pub provider_reason: Option<String>,
    pub receipt: EvidenceRef,
    pub schema: String,
    pub status: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProviderContractError {
    #[error("invalid provider contract field: {0}")]
    Invalid(&'static str),
    #[error("provider binding mismatch")]
    BindingMismatch,
}

impl ProviderRequest {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema != REQUEST_SCHEMA
            || self.deadline_ms == 0
            || self.deadline_ms > MAX_DURATION_MS
        {
            return Err(ProviderContractError::Invalid("schema/deadlineMs"));
        }
        self.identity.validate()?;
        self.evidence.validate(self.operation)?;
        validate_receipt_paths(self.operation, &self.evidence.receipt_rel_paths)?;
        let data = object(&self.operation_data)?;
        exact_keys(data, request_keys(self.operation))?;
        validate_request_data(self, data)
    }
}

impl ProviderResponse {
    pub fn validate_for(&self, request: &ProviderRequest) -> Result<(), ProviderContractError> {
        request.validate()?;
        if self.schema != RESPONSE_SCHEMA
            || self.operation != request.operation
            || self.identity != request.identity
            || !matches!(
                self.status.as_str(),
                "done" | "running" | "blocked" | "failed"
            )
            || (self.ok != self.provider_reason.is_none())
        {
            return Err(ProviderContractError::Invalid("response envelope"));
        }
        validate_response_status(self)?;
        self.receipt
            .validate()
            .map_err(|_| ProviderContractError::Invalid("receipt"))?;
        if self.receipt.path != request.evidence.receipt_rel_paths.primary
            || self.receipt.media_type != EvidenceMediaType::Json
        {
            return Err(ProviderContractError::Invalid("receipt"));
        }
        let data = object(&self.operation_data)?;
        exact_keys(data, response_keys(self.operation, self.ok))?;
        validate_response_data(self, request, data)
    }
}

impl ProviderIdentity {
    fn validate(&self) -> Result<(), ProviderContractError> {
        optional_text(&self.cohort, validate_cohort, "identity.cohort")?;
        required_text(
            &self.operation_id,
            validate_operation_id,
            "identity.operationId",
        )?;
        optional_text(&self.request_id, validate_request_id, "identity.requestId")?;
        optional_text(&self.run_id, validate_run_id, "identity.runId")?;
        optional_text(&self.session_id, validate_session_id, "identity.sessionId")?;
        required_text(&self.slot_id, validate_slot_id, "identity.slotId")?;
        Ok(())
    }
}

impl ProviderEvidencePaths {
    fn validate(&self, operation: ProviderOperation) -> Result<(), ProviderContractError> {
        for (value, expected, field) in [
            (
                &self.cdp_rel_path,
                "cdp.sanitized.json",
                "evidence.cdpRelPath",
            ),
            (
                &self.dom_rel_path,
                "dom.sanitized.json",
                "evidence.domRelPath",
            ),
            (
                &self.screenshot_rel_path,
                "screenshot.privacy-crop.png",
                "evidence.screenshotRelPath",
            ),
            (
                &self.receipt_rel_paths.primary,
                "provider-receipt.json",
                "evidence.receiptRelPaths.primary",
            ),
        ] {
            required_text(value, validate_safe_rel_path, field)?;
            if value != expected {
                return Err(ProviderContractError::Invalid(field));
            }
        }
        let expected = match operation {
            ProviderOperation::SendClick => (
                Some("send.pre-click.receipt.json"),
                Some("send.post-click.receipt.json"),
                None,
            ),
            ProviderOperation::SendReconcile => (
                Some("send.pre-click.receipt.json"),
                None,
                Some("send.reconcile.receipt.json"),
            ),
            _ => (None, None, None),
        };
        for (value, expected, field) in [
            (
                &self.receipt_rel_paths.pre_click,
                expected.0,
                "evidence.receiptRelPaths.preClick",
            ),
            (
                &self.receipt_rel_paths.post_click,
                expected.1,
                "evidence.receiptRelPaths.postClick",
            ),
            (
                &self.receipt_rel_paths.reconcile,
                expected.2,
                "evidence.receiptRelPaths.reconcile",
            ),
        ] {
            if value.as_deref() != expected {
                return Err(ProviderContractError::Invalid(field));
            }
            if let Some(value) = value {
                required_text(value, validate_safe_rel_path, field)?;
            }
        }
        Ok(())
    }
}

impl ProviderAttachmentSet {
    fn validate(&self) -> Result<(), ProviderContractError> {
        if usize::from(self.count) != self.records.len() || self.records.len() > MAX_ATTACHMENTS {
            return Err(ProviderContractError::Invalid("attachmentSet.count"));
        }
        required_text(&self.set_sha256, validate_h256, "attachmentSet.setSha256")?;
        for (index, record) in self.records.iter().enumerate() {
            if usize::from(record.ordinal) != index {
                return Err(ProviderContractError::Invalid("attachmentSet.ordinal"));
            }
            required_text(
                &record.container_rel_path,
                validate_safe_rel_path,
                "attachmentSet.containerRelPath",
            )?;
            required_text(
                &record.source_sha256,
                validate_h256,
                "attachmentSet.sourceSha256",
            )?;
            validate_byte_count(record.size_bytes)
                .map_err(|_| ProviderContractError::Invalid("attachmentSet.sizeBytes"))?;
            if record.media_type.is_empty()
                || record.media_type.len() > 4_096
                || record.media_type.contains('\0')
            {
                return Err(ProviderContractError::Invalid("attachmentSet.mediaType"));
            }
        }
        Ok(())
    }
}

impl ClearedChip {
    fn validate(&self) -> Result<(), ProviderContractError> {
        required_text(
            &self.chip_stable_key,
            validate_h256,
            "clearedChip.chipStableKey",
        )?;
        optional_text(&self.digest, validate_h256, "clearedChip.digest")?;
        self.cleared
            .then_some(())
            .ok_or(ProviderContractError::Invalid("clearedChip.cleared"))
    }
}

fn validate_request_data(
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    match request.operation {
        ProviderOperation::Status => {
            if text(data, "expectedSlotId", "expectedSlotId")? != request.identity.slot_id
                || integer(data, "probeAttempt", "probeAttempt")? > 1
            {
                return Err(ProviderContractError::Invalid("status request"));
            }
        }
        ProviderOperation::CaptureRoot => {
            let model: Model = parse(data.get("requestedModel"), "requestedModel")?;
            let effort: Effort = parse(data.get("requestedEffort"), "requestedEffort")?;
            validate_model_tuple(&model, &effort)
                .map_err(|_| ProviderContractError::Invalid("model/effort tuple"))?;
            if integer(data, "rediscoveryAttempt", "rediscoveryAttempt")? > 2 {
                return Err(ProviderContractError::Invalid("rediscoveryAttempt"));
            }
        }
        ProviderOperation::EnsureModel => {
            let binding = page_binding(data, request)?;
            let model: Model = parse(data.get("requestedModel"), "requestedModel")?;
            let effort: Effort = parse(data.get("requestedEffort"), "requestedEffort")?;
            validate_model_tuple(&model, &effort)
                .map_err(|_| ProviderContractError::Invalid("model/effort tuple"))?;
            if integer(data, "pickerOpenBudget", "pickerOpenBudget")? != 1
                || integer(data, "stabilizationMs", "stabilizationMs")? != 500
            {
                return Err(ProviderContractError::Invalid("ensure-model literals"));
            }
            validate_page_identity(&binding, &request.identity)?;
        }
        ProviderOperation::UploadOnly => {
            page_binding(data, request)?;
            let attachments: ProviderAttachmentSet =
                parse(data.get("attachmentSet"), "attachmentSet")?;
            attachments.validate()?;
            id_value(data, "uploadAttemptId", validate_operation_id)?;
            if integer(data, "retryIndex", "retryIndex")? > 1 {
                return Err(ProviderContractError::Invalid("retryIndex"));
            }
        }
        ProviderOperation::ClearUpload => {
            page_binding(data, request)?;
            id_value(data, "uploadAttemptId", validate_operation_id)?;
            id_value(data, "clearAttemptId", validate_operation_id)?;
            let chips: Vec<ChipProof> = parse(data.get("staleChips"), "staleChips")?;
            if !(1..=MAX_ATTACHMENTS).contains(&chips.len()) {
                return Err(ProviderContractError::Invalid("staleChips"));
            }
            chips.iter().try_for_each(|proof| {
                proof
                    .validate()
                    .map_err(|_| ProviderContractError::Invalid("staleChips"))
            })?;
            unique_chip_keys(&chips, "staleChips")?;
        }
        ProviderOperation::SendClick => {
            let binding = page_binding(data, request)?;
            id_value(data, "sendAttemptId", validate_operation_id)?;
            let proof: UploadProof = parse(data.get("uploadProof"), "uploadProof")?;
            proof
                .validate()
                .map_err(|_| ProviderContractError::Invalid("uploadProof"))?;
            let prompt: PromptInput = parse(data.get("promptInput"), "promptInput")?;
            validate_prompt(&prompt, request.identity.run_id.as_deref())?;
            if integer(data, "clickBudget", "clickBudget")? != 1
                || proof.stale_chips.len() > MAX_ATTACHMENTS
            {
                return Err(ProviderContractError::Invalid("send-click request"));
            }
            validate_page_identity(&binding, &request.identity)?;
        }
        ProviderOperation::SendReconcile => {
            let binding = page_binding(data, request)?;
            let attempt = id_value(data, "sendAttemptId", validate_operation_id)?;
            let receipt: SendReceipt = parse(data.get("preClickReceipt"), "preClickReceipt")?;
            receipt
                .validate()
                .map_err(|_| ProviderContractError::Invalid("preClickReceipt"))?;
            if receipt.kind != SendReceiptKind::PreClick
                || receipt.send_attempt_id != attempt
                || receipt.page_binding != binding
            {
                return Err(ProviderContractError::Invalid("preClickReceipt binding"));
            }
        }
        ProviderOperation::SessionRebind => {
            if !matches!(
                text(data, "operationKind", "operationKind")?,
                "poll" | "show" | "resume" | "download"
            ) || integer(data, "navigationAttemptLimit", "navigationAttemptLimit")?
                != u64::from(NAVIGATION_ATTEMPT_LIMIT)
                || integer(data, "hydrationDeadlineMs", "hydrationDeadlineMs")?
                    != HYDRATION_DEADLINE_MS
            {
                return Err(ProviderContractError::Invalid("session-rebind request"));
            }
            let expected: SessionRebindExpectation = parse(data.get("expectation"), "expectation")?;
            expected
                .validate()
                .map_err(|_| ProviderContractError::Invalid("expectation"))?;
            validate_expectation_identity(&expected, &request.identity)?;
        }
        ProviderOperation::Poll => {
            session_echo(data, "expected", request)?;
            id_value(data, "pollAttemptId", validate_operation_id)?;
            if !(1..=10_800).contains(&integer(data, "pollTimeoutSeconds", "pollTimeoutSeconds")?) {
                return Err(ProviderContractError::Invalid("pollTimeoutSeconds"));
            }
            parse_artifact_expectation(data.get("artifactExpectation"))?;
        }
        ProviderOperation::ArtifactDiscover => {
            session_echo(data, "expected", request)?;
            id_value(data, "artifactClaimId", validate_artifact_claim_id)?;
            id_value(data, "terminalAssistantTurnId", validate_turn_id)?;
            parse_artifact_expectation(data.get("expectation"))?;
        }
        ProviderOperation::ArtifactClickSave => {
            let expected = session_echo(data, "expected", request)?;
            let claim_id = id_value(data, "artifactClaimId", validate_artifact_claim_id)?;
            let turn_id = id_value(data, "terminalAssistantTurnId", validate_turn_id)?;
            let control: ArtifactControl = parse(data.get("control"), "control")?;
            control
                .validate_for_turn(turn_id)
                .map_err(|_| ProviderContractError::Invalid("control"))?;
            let baseline: ArtifactBaseline = parse(data.get("baseline"), "baseline")?;
            baseline
                .validate()
                .map_err(|_| ProviderContractError::Invalid("baseline"))?;
            if integer(data, "controlIndex", "controlIndex")? > 63 {
                return Err(ProviderContractError::Invalid("controlIndex"));
            }
            id_value(data, "hostSaveDirectory", validate_safe_rel_path)?;
            validate_session_identity(&expected, &request.identity)?;
            let _ = claim_id;
        }
    }
    Ok(())
}

fn validate_response_status(response: &ProviderResponse) -> Result<(), ProviderContractError> {
    if response.ok {
        if !matches!(response.status.as_str(), "done" | "running") {
            return Err(ProviderContractError::Invalid("response status"));
        }
    } else {
        let reason = response
            .provider_reason
            .as_deref()
            .ok_or(ProviderContractError::Invalid("providerReason"))?;
        let blocked = matches!(
            reason,
            "session.provider_limit"
                | "session.login_required"
                | "session.subscription_required"
                | "provider.limit"
                | "provider.login_required"
                | "provider.subscription_required"
        );
        if response.status != if blocked { "blocked" } else { "failed" } {
            return Err(ProviderContractError::Invalid("response status"));
        }
    }
    Ok(())
}

fn validate_response_data(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    match response.operation {
        ProviderOperation::Status => validate_status_response(response, data),
        ProviderOperation::CaptureRoot => validate_capture_response(response, request, data),
        ProviderOperation::EnsureModel => validate_model_response(response, request, data),
        ProviderOperation::UploadOnly => validate_upload_response(response, request, data),
        ProviderOperation::ClearUpload => validate_clear_response(response, request, data),
        ProviderOperation::SendClick | ProviderOperation::SendReconcile => {
            validate_send_response(response, request, data)
        }
        ProviderOperation::SessionRebind => validate_rebind_response(response, request, data),
        ProviderOperation::Poll => validate_poll_response(response, request, data),
        ProviderOperation::ArtifactDiscover => validate_discover_response(response, request, data),
        ProviderOperation::ArtifactClickSave => validate_download_response(response, request, data),
    }
}

fn validate_status_response(
    response: &ProviderResponse,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    if HealthStatus::parse(text(data, "healthStatus", "healthStatus")?).is_none()
        || !matches!(
            text(data, "dockerStatus", "dockerStatus")?,
            "running" | "exited" | "missing" | "starting" | "stopping" | "unknown"
        )
        || !matches!(
            text(data, "modelLabel", "modelLabel")?,
            "pro" | "non_pro" | "unknown"
        )
    {
        return Err(ProviderContractError::Invalid("status observation"));
    }
    optional_integer(data, "retryAfterMs", "retryAfterMs")?;
    boolean(data, "composerReady", "composerReady")?;
    if response.ok && response.status != "done" {
        return Err(ProviderContractError::Invalid("status response status"));
    }
    if !response.ok
        && !matches!(
            response.provider_reason.as_deref(),
            Some("probe.timeout" | "probe.unreachable")
        )
    {
        return Err(ProviderContractError::Invalid("status providerReason"));
    }
    Ok(())
}

fn validate_capture_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    if response.ok {
        let candidate: RootBindingCandidate =
            parse(data.get("rootBindingCandidate"), "rootBindingCandidate")?;
        candidate
            .validate()
            .map_err(|_| ProviderContractError::Invalid("rootBindingCandidate"))?;
        if candidate.operation_id != request.identity.operation_id
            || !data.get("failureProof").is_some_and(Value::is_null)
            || response.status != "done"
        {
            return Err(ProviderContractError::Invalid("capture success"));
        }
    } else {
        if !data.get("rootBindingCandidate").is_some_and(Value::is_null) {
            return Err(ProviderContractError::Invalid("capture failure"));
        }
        match response.provider_reason.as_deref() {
            Some("capture.ambiguous") => {
                let proof: FailureProof = parse(data.get("failureProof"), "failureProof")?;
                proof
                    .validate()
                    .map_err(|_| ProviderContractError::Invalid("failureProof"))?;
                if proof.reason != "capture.ambiguous" {
                    return Err(ProviderContractError::Invalid("failureProof.reason"));
                }
            }
            Some("capture.timeout") if data.get("failureProof").is_some_and(Value::is_null) => {}
            _ => return Err(ProviderContractError::Invalid("capture providerReason")),
        }
    }
    Ok(())
}

fn validate_model_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected_binding: PageBindingEcho = parse(request_data.get("pageBinding"), "pageBinding")?;
    let expected_model: Model = parse(request_data.get("requestedModel"), "requestedModel")?;
    let expected_effort: Effort = parse(request_data.get("requestedEffort"), "requestedEffort")?;
    let observed = optional_binding(data.get("observedPageBinding"))?;
    let binding_mismatch = response.provider_reason.as_deref() == Some("binding.mismatch");
    if binding_mismatch {
        if observed
            .as_ref()
            .is_none_or(|value| value == &expected_binding)
        {
            return Err(ProviderContractError::Invalid(
                "ensure-model binding mismatch echo",
            ));
        }
    } else {
        compare_binding(observed.as_ref(), &expected_binding)?;
    }
    if response.ok {
        let model: ModelProof = parse(data.get("modelProof"), "modelProof")?;
        let effort: EffortProof = parse(data.get("effortProof"), "effortProof")?;
        model
            .validate()
            .map_err(|_| ProviderContractError::Invalid("modelProof"))?;
        effort
            .validate()
            .map_err(|_| ProviderContractError::Invalid("effortProof"))?;
        if model.requested != expected_model
            || model.observed != expected_model
            || effort.requested != expected_effort
            || effort.observed != expected_effort
            || observed.is_none()
            || !data.get("failureProof").is_some_and(Value::is_null)
            || response.status != "done"
        {
            return Err(ProviderContractError::Invalid("ensure-model success"));
        }
    } else if matches!(
        response.provider_reason.as_deref(),
        Some(
            "picker.model_absent"
                | "picker.effort_absent"
                | "picker.control_drift"
                | "picker.selection_timeout"
                | "picker.reverify_mismatch"
                | "capture.ambiguous"
        )
    ) {
        let proof: FailureProof = parse(data.get("failureProof"), "failureProof")?;
        proof
            .validate()
            .map_err(|_| ProviderContractError::Invalid("failureProof"))?;
        if data.get("modelProof").is_none_or(|value| !value.is_null())
            || data.get("effortProof").is_none_or(|value| !value.is_null())
            || proof.reason != response.provider_reason.as_deref().unwrap_or_default()
            || observed.is_none()
        {
            return Err(ProviderContractError::Invalid(
                "ensure-model picker failure",
            ));
        }
    } else if matches!(
        response.provider_reason.as_deref(),
        Some("provider.schema_drift" | "contract.invalid_provider_envelope" | "binding.mismatch")
    ) {
        if ["modelProof", "effortProof", "failureProof"]
            .iter()
            .any(|key| data.get(*key).is_none_or(|value| !value.is_null()))
        {
            return Err(ProviderContractError::Invalid(
                "ensure-model invocation failure",
            ));
        }
    } else {
        return Err(ProviderContractError::Invalid(
            "ensure-model providerReason",
        ));
    }
    Ok(())
}

fn validate_upload_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected_binding: PageBindingEcho = parse(request_data.get("pageBinding"), "pageBinding")?;
    let attachments: ProviderAttachmentSet =
        parse(request_data.get("attachmentSet"), "attachmentSet")?;
    let attempt = text(request_data, "uploadAttemptId", "uploadAttemptId")?;
    let retry = integer(request_data, "retryIndex", "retryIndex")? as u8;
    let observed = optional_binding(data.get("observedPageBinding"))?;
    compare_binding(observed.as_ref(), &expected_binding)?;
    let proof = optional_parse::<UploadProof>(data.get("uploadProof"), "uploadProof")?;
    if let Some(proof) = &proof {
        proof
            .validate()
            .map_err(|_| ProviderContractError::Invalid("uploadProof"))?;
        if proof.upload_attempt_id != attempt
            || proof.retry_index != retry
            || proof.expected_set_sha256 != attachments.set_sha256
        {
            return Err(ProviderContractError::Invalid("uploadProof binding"));
        }
    }
    let failure_reason = optional_string(data, "failureReason", "failureReason")?;
    if response.ok {
        let proof = proof.ok_or(ProviderContractError::Invalid("uploadProof"))?;
        if failure_reason.is_some()
            || observed.is_none()
            || !proof.all_expected_complete
            || !proof.stale_chips.is_empty()
            || proof.visible_current_chips.len() != usize::from(attachments.count)
            || response.status != "done"
        {
            return Err(ProviderContractError::Invalid("upload success"));
        }
    } else if response.provider_reason.as_deref() == Some("upload.stale_chip_mismatch") {
        let proof = proof.ok_or(ProviderContractError::Invalid("uploadProof"))?;
        if retry != 0
            || proof.stale_chips.is_empty()
            || observed.is_none()
            || failure_reason.as_deref() != response.provider_reason.as_deref()
        {
            return Err(ProviderContractError::Invalid("upload mismatch"));
        }
    } else if !matches!(
        response.provider_reason.as_deref(),
        Some("upload.stale_chip_uncleared" | "upload.incomplete" | "upload.chip_removal_failed")
    ) || proof.is_some()
        || failure_reason.as_deref() != response.provider_reason.as_deref()
    {
        return Err(ProviderContractError::Invalid("upload failure"));
    }
    Ok(())
}

fn validate_clear_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected_binding: PageBindingEcho = parse(request_data.get("pageBinding"), "pageBinding")?;
    let requested: Vec<ChipProof> = parse(request_data.get("staleChips"), "staleChips")?;
    let clear_id = text(request_data, "clearAttemptId", "clearAttemptId")?;
    if text(data, "clearAttemptId", "clearAttemptId")? != clear_id {
        return Err(ProviderContractError::Invalid("clearAttemptId echo"));
    }
    let observed = optional_binding(data.get("observedPageBinding"))?;
    compare_binding(observed.as_ref(), &expected_binding)?;
    let cleared: Vec<ClearedChip> = parse(data.get("clearedChips"), "clearedChips")?;
    if cleared.len() > MAX_ATTACHMENTS {
        return Err(ProviderContractError::Invalid("clearedChips"));
    }
    cleared.iter().try_for_each(ClearedChip::validate)?;
    if response.ok {
        if observed.is_none()
            || cleared.len() != requested.len()
            || chip_pairs(&cleared) != requested_chip_pairs(&requested)
            || response.status != "done"
        {
            return Err(ProviderContractError::Invalid("clear-upload success"));
        }
    } else {
        if response.provider_reason.as_deref() != Some("upload.chip_removal_failed")
            || optional_string(data, "failureReason", "failureReason")?.as_deref()
                != response.provider_reason.as_deref()
        {
            return Err(ProviderContractError::Invalid("clear-upload failure"));
        }
        let attempted = array(data, "attemptedChipKeys", "attemptedChipKeys")?;
        if !(1..=MAX_ATTACHMENTS).contains(&attempted.len()) {
            return Err(ProviderContractError::Invalid("attemptedChipKeys"));
        }
        attempted.iter().try_for_each(|value| {
            let value = value
                .as_str()
                .ok_or(ProviderContractError::Invalid("attemptedChipKeys"))?;
            required_text(value, validate_h256, "attemptedChipKeys")
        })?;
    }
    Ok(())
}

fn validate_send_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let binding: PageBindingEcho = parse(request_data.get("pageBinding"), "pageBinding")?;
    let attempt = text(request_data, "sendAttemptId", "sendAttemptId")?;
    let pre_click: SendReceipt = parse(data.get("preClickReceipt"), "preClickReceipt")?;
    pre_click
        .validate()
        .map_err(|_| ProviderContractError::Invalid("preClickReceipt"))?;
    if pre_click.kind != SendReceiptKind::PreClick
        || pre_click.send_attempt_id != attempt
        || pre_click.page_binding != binding
    {
        return Err(ProviderContractError::Invalid("preClickReceipt binding"));
    }
    match request.operation {
        ProviderOperation::SendClick => {
            let prompt: PromptInput = parse(request_data.get("promptInput"), "promptInput")?;
            if pre_click.prompt_sha256 != prompt.sha256 {
                return Err(ProviderContractError::Invalid("preClickReceipt prompt"));
            }
        }
        ProviderOperation::SendReconcile => {
            let expected: SendReceipt =
                parse(request_data.get("preClickReceipt"), "preClickReceipt")?;
            if pre_click != expected {
                return Err(ProviderContractError::Invalid("preClickReceipt echo"));
            }
        }
        _ => unreachable!("validated operation"),
    }
    let terminal =
        optional_parse::<SendReceipt>(data.get("terminalSendReceipt"), "terminalSendReceipt")?;
    if let Some(terminal) = &terminal {
        validate_receipt_pair(&pre_click, terminal, &binding)
            .map_err(|_| ProviderContractError::Invalid("terminalSendReceipt"))?;
        let expected_kind = match request.operation {
            ProviderOperation::SendClick => SendReceiptKind::PostClick,
            ProviderOperation::SendReconcile => SendReceiptKind::ReconciledTurnStart,
            _ => unreachable!("validated operation"),
        };
        if terminal.kind != expected_kind {
            return Err(ProviderContractError::Invalid("terminalSendReceipt kind"));
        }
    }
    let observed = optional_binding(data.get("observedPageBinding"))?;
    compare_binding(observed.as_ref(), &binding)?;
    if response.ok {
        if terminal.is_none() || observed.is_none() || response.status != "done" {
            return Err(ProviderContractError::Invalid("send success"));
        }
    } else if !matches!(
        response.provider_reason.as_deref(),
        Some("send.turn_not_proven" | "send.click_timeout")
    ) {
        return Err(ProviderContractError::Invalid("send failure"));
    }
    Ok(())
}

fn validate_rebind_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected: SessionRebindExpectation = parse(request_data.get("expectation"), "expectation")?;
    let echoed: SessionRebindExpectation = parse(data.get("expectation"), "expectation")?;
    if expected != echoed {
        return Err(ProviderContractError::Invalid("expectation echo"));
    }
    if response.ok {
        let observed: SessionEcho = parse(data.get("observedEcho"), "observedEcho")?;
        let generation = integer(data, "pageBindingGeneration", "pageBindingGeneration")?;
        let observations: Vec<HydrationObservation> =
            parse(data.get("hydrationObservations"), "hydrationObservations")?;
        let terminal: Option<TerminalAnswerObservation> =
            optional_parse(data.get("terminalAnswer"), "terminalAnswer")?;
        if !data.get("failureReason").is_some_and(Value::is_null)
            || generation == 0
            || generation > u64::from(u16::MAX)
            || response.status != "done" && response.status != "running"
        {
            return Err(ProviderContractError::Invalid("session-rebind success"));
        }
        let proof = RebindProof {
            expectation: echoed,
            observed_echo: observed,
            page_binding_generation: generation as u16,
            hydration: HydrationTrace { observations },
            terminal_answer: terminal,
        };
        proof
            .validate(&expected)
            .map_err(|_| ProviderContractError::Invalid("session-rebind proof"))?;
    } else {
        if !data
            .get("pageBindingGeneration")
            .is_some_and(Value::is_null)
            || optional_string(data, "failureReason", "failureReason")?.as_deref()
                != response.provider_reason.as_deref()
            || !is_session_failure(response.provider_reason.as_deref())
        {
            return Err(ProviderContractError::Invalid("session-rebind failure"));
        }
        let observations: Vec<HydrationObservation> =
            parse(data.get("hydrationObservations"), "hydrationObservations")?;
        if observations.len() > 50 {
            return Err(ProviderContractError::Invalid("hydrationObservations"));
        }
        observations.iter().try_for_each(|value| {
            value
                .validate()
                .map_err(|_| ProviderContractError::Invalid("hydrationObservations"))
        })?;
        let observed = optional_parse::<SessionEcho>(data.get("observedEcho"), "observedEcho")?;
        if let Some(observed) = &observed {
            observed
                .validate()
                .map_err(|_| ProviderContractError::Invalid("observedEcho"))?;
        }
        match response.provider_reason.as_deref() {
            Some("session.url_rejected_root") => {
                if observed.is_some() || !observations.is_empty() {
                    return Err(ProviderContractError::Invalid("root observedEcho"));
                }
            }
            Some("session.url_rejected_mismatch") => {
                let observed = observed
                    .as_ref()
                    .ok_or(ProviderContractError::Invalid("mismatch observedEcho"))?;
                if crate::session_rebind::validate_observed_echo(&expected, observed).is_ok() {
                    return Err(ProviderContractError::Invalid("mismatch observedEcho"));
                }
            }
            _ => {
                if let Some(observed) = &observed {
                    crate::session_rebind::validate_observed_echo(&expected, observed)
                        .map_err(|_| ProviderContractError::BindingMismatch)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_poll_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let requested: SessionEcho = parse(request_data.get("expected"), "expected")?;
    let echoed: SessionEcho = parse(data.get("expected"), "expected")?;
    if echoed != requested {
        return Err(ProviderContractError::Invalid("expected echo"));
    }
    let observed = optional_parse::<SessionEcho>(data.get("observedEcho"), "observedEcho")?;
    if let Some(observed) = &observed {
        observed
            .validate()
            .map_err(|_| ProviderContractError::Invalid("observedEcho"))?;
    }
    if response.ok {
        let observed = observed
            .as_ref()
            .ok_or(ProviderContractError::Invalid("observedEcho"))?;
        validate_expected_echo(&requested, observed)?;
        match text(data, "pollState", "pollState")? {
            "running" => {
                if response.status != "running"
                    || !all_null(
                        data,
                        &[
                            "answerSha256",
                            "answerSizeBytes",
                            "answerRelPath",
                            "terminalAssistantTurnId",
                            "bottomProof",
                        ],
                    )
                {
                    return Err(ProviderContractError::Invalid("poll running"));
                }
            }
            "terminal" => {
                if response.status != "done" {
                    return Err(ProviderContractError::Invalid("poll terminal status"));
                }
                let answer_sha = id_value(data, "answerSha256", validate_h256)?;
                validate_byte_count(integer(data, "answerSizeBytes", "answerSizeBytes")?)
                    .map_err(|_| ProviderContractError::Invalid("answerSizeBytes"))?;
                id_value(data, "answerRelPath", validate_safe_rel_path)?;
                let turn = id_value(data, "terminalAssistantTurnId", validate_turn_id)?;
                if observed.terminal_answer_sha256.as_deref() != Some(answer_sha)
                    || observed.visible_assistant_turn_id.as_deref() != Some(turn)
                {
                    return Err(ProviderContractError::Invalid("poll answer binding"));
                }
                if let Some(proof) =
                    optional_parse::<BottomProof>(data.get("bottomProof"), "bottomProof")?
                {
                    proof
                        .validate()
                        .map_err(|_| ProviderContractError::Invalid("bottomProof"))?;
                }
            }
            _ => return Err(ProviderContractError::Invalid("pollState")),
        }
    } else {
        if text(data, "pollState", "pollState")? != "failed"
            || !all_null(
                data,
                &[
                    "answerSha256",
                    "answerSizeBytes",
                    "answerRelPath",
                    "terminalAssistantTurnId",
                    "bottomProof",
                ],
            )
            || !is_session_failure(response.provider_reason.as_deref())
        {
            return Err(ProviderContractError::Invalid("poll failure"));
        }
        validate_failure_echo_rule(
            response.provider_reason.as_deref(),
            observed.as_ref(),
            &requested,
        )?;
    }
    Ok(())
}

fn validate_discover_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected: SessionEcho = parse(request_data.get("expected"), "expected")?;
    let claim_id = text(request_data, "artifactClaimId", "artifactClaimId")?;
    let turn_id = text(
        request_data,
        "terminalAssistantTurnId",
        "terminalAssistantTurnId",
    )?;
    let observed = optional_parse::<SessionEcho>(data.get("observedEcho"), "observedEcho")?;
    if let Some(observed) = &observed {
        observed
            .validate()
            .map_err(|_| ProviderContractError::Invalid("observedEcho"))?;
    }
    let controls: Vec<ArtifactControl> = parse(data.get("controls"), "controls")?;
    if controls.len() > 64 {
        return Err(ProviderContractError::Invalid("controls"));
    }
    controls.iter().try_for_each(|control| {
        control
            .validate_for_turn(turn_id)
            .map_err(|_| ProviderContractError::Invalid("controls"))
    })?;
    let unique = controls
        .iter()
        .map(|control| control.control_id.as_str())
        .collect::<BTreeSet<_>>();
    if unique.len() != controls.len() {
        return Err(ProviderContractError::Invalid("duplicate controls"));
    }
    if response.ok {
        let observed = observed
            .as_ref()
            .ok_or(ProviderContractError::Invalid("observedEcho"))?;
        validate_expected_echo(&expected, observed)?;
        let bottom: BottomProof = parse(data.get("bottomProof"), "bottomProof")?;
        bottom
            .validate()
            .map_err(|_| ProviderContractError::Invalid("bottomProof"))?;
        if !data.get("failureReason").is_some_and(Value::is_null) {
            return Err(ProviderContractError::Invalid("failureReason"));
        }
        let zero =
            optional_parse::<ZeroControlProof>(data.get("zeroControlProof"), "zeroControlProof")?;
        if controls.is_empty() {
            let zero = zero.ok_or(ProviderContractError::Invalid("zeroControlProof"))?;
            zero.validate_for(claim_id, turn_id)
                .map_err(|_| ProviderContractError::Invalid("zeroControlProof"))?;
            if zero.bottom_proof != bottom {
                return Err(ProviderContractError::Invalid(
                    "zeroControlProof.bottomProof",
                ));
            }
        } else if zero.is_some() {
            return Err(ProviderContractError::Invalid(
                "zeroControlProof nullability",
            ));
        }
    } else {
        let observed = observed
            .as_ref()
            .ok_or(ProviderContractError::Invalid("observedEcho"))?;
        if !controls.is_empty()
            || !data.get("bottomProof").is_some_and(Value::is_null)
            || !data.get("zeroControlProof").is_some_and(Value::is_null)
            || !matches!(
                response.provider_reason.as_deref(),
                Some("artifact.controls_ambiguous" | "artifact.bottom_unverified")
            )
            || optional_string(data, "failureReason", "failureReason")?.as_deref()
                != response.provider_reason.as_deref()
        {
            return Err(ProviderContractError::Invalid("artifact-discover failure"));
        }
        validate_expected_echo(&expected, observed)?;
    }
    Ok(())
}

fn validate_download_response(
    response: &ProviderResponse,
    request: &ProviderRequest,
    data: &Map<String, Value>,
) -> Result<(), ProviderContractError> {
    let request_data = object(&request.operation_data)?;
    let expected: SessionEcho = parse(request_data.get("expected"), "expected")?;
    let claim_id = text(request_data, "artifactClaimId", "artifactClaimId")?;
    let turn_id = text(
        request_data,
        "terminalAssistantTurnId",
        "terminalAssistantTurnId",
    )?;
    let control: ArtifactControl = parse(request_data.get("control"), "control")?;
    let host_directory = text(request_data, "hostSaveDirectory", "hostSaveDirectory")?;
    let observed = optional_parse::<SessionEcho>(data.get("observedEcho"), "observedEcho")?;
    if let Some(observed) = &observed {
        observed
            .validate()
            .map_err(|_| ProviderContractError::Invalid("observedEcho"))?;
    }
    if response.ok {
        let observed = observed
            .as_ref()
            .ok_or(ProviderContractError::Invalid("observedEcho"))?;
        validate_expected_echo(&expected, observed)?;
        let receipt: PlaywrightDownloadReceipt =
            parse(data.get("downloadReceipt"), "downloadReceipt")?;
        receipt
            .validate_for(&expected, claim_id, &control, turn_id)
            .map_err(|_| ProviderContractError::Invalid("downloadReceipt"))?;
        let parent = std::path::Path::new(&receipt.host_saved_rel_path)
            .parent()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if parent != host_directory
            || !data.get("failureReason").is_some_and(Value::is_null)
            || response.status != "done"
        {
            return Err(ProviderContractError::Invalid(
                "artifact-click-save success",
            ));
        }
    } else {
        let observed = observed
            .as_ref()
            .ok_or(ProviderContractError::Invalid("observedEcho"))?;
        if !data.get("downloadReceipt").is_some_and(Value::is_null)
            || !matches!(
                response.provider_reason.as_deref(),
                Some(
                    "artifact.download_timeout"
                        | "artifact.event_unrecoverable"
                        | "artifact.integrity_failed"
                        | "artifact.path_unsafe"
                )
            )
            || optional_string(data, "failureReason", "failureReason")?.as_deref()
                != response.provider_reason.as_deref()
        {
            return Err(ProviderContractError::Invalid(
                "artifact-click-save failure",
            ));
        }
        validate_expected_echo(&expected, observed)?;
    }
    Ok(())
}

fn page_binding(
    data: &Map<String, Value>,
    request: &ProviderRequest,
) -> Result<PageBindingEcho, ProviderContractError> {
    let binding: PageBindingEcho = parse(data.get("pageBinding"), "pageBinding")?;
    binding
        .validate()
        .map_err(|_| ProviderContractError::Invalid("pageBinding"))?;
    validate_page_identity(&binding, &request.identity)?;
    Ok(binding)
}

fn session_echo(
    data: &Map<String, Value>,
    key: &'static str,
    request: &ProviderRequest,
) -> Result<SessionEcho, ProviderContractError> {
    let echo: SessionEcho = parse(data.get(key), key)?;
    echo.validate()
        .map_err(|_| ProviderContractError::Invalid(key))?;
    validate_session_identity(&echo, &request.identity)?;
    Ok(echo)
}

fn validate_page_identity(
    binding: &PageBindingEcho,
    identity: &ProviderIdentity,
) -> Result<(), ProviderContractError> {
    if binding.slot_id != identity.slot_id
        || identity
            .cohort
            .as_ref()
            .is_some_and(|cohort| &binding.cohort != cohort)
    {
        return Err(ProviderContractError::BindingMismatch);
    }
    Ok(())
}

fn validate_session_identity(
    echo: &SessionEcho,
    identity: &ProviderIdentity,
) -> Result<(), ProviderContractError> {
    validate_page_identity(&echo.page_binding, identity)?;
    if identity
        .session_id
        .as_ref()
        .is_some_and(|value| &echo.session_id != value)
        || identity
            .request_id
            .as_ref()
            .is_some_and(|value| echo.request_id.as_ref() != Some(value))
        || identity
            .run_id
            .as_ref()
            .is_some_and(|value| echo.run_id.as_ref() != Some(value))
    {
        return Err(ProviderContractError::BindingMismatch);
    }
    Ok(())
}

fn validate_expectation_identity(
    expectation: &SessionRebindExpectation,
    identity: &ProviderIdentity,
) -> Result<(), ProviderContractError> {
    if expectation.slot_id != identity.slot_id
        || identity
            .cohort
            .as_ref()
            .is_some_and(|value| &expectation.cohort != value)
        || identity
            .session_id
            .as_ref()
            .is_some_and(|value| &expectation.session_id != value)
        || identity
            .request_id
            .as_ref()
            .is_some_and(|value| expectation.request_id.as_ref() != Some(value))
        || identity
            .run_id
            .as_ref()
            .is_some_and(|value| expectation.run_id.as_ref() != Some(value))
    {
        return Err(ProviderContractError::BindingMismatch);
    }
    Ok(())
}

fn validate_prompt(
    prompt: &PromptInput,
    run_id: Option<&str>,
) -> Result<(), ProviderContractError> {
    required_text(
        &prompt.container_rel_path,
        validate_safe_rel_path,
        "promptInput.containerRelPath",
    )?;
    required_text(&prompt.sha256, validate_h256, "promptInput.sha256")?;
    validate_byte_count(prompt.size_bytes)
        .map_err(|_| ProviderContractError::Invalid("promptInput.sizeBytes"))?;
    if run_id.is_some_and(|run_id| prompt.container_rel_path != format!("{run_id}/prompt.txt")) {
        return Err(ProviderContractError::Invalid(
            "promptInput.containerRelPath",
        ));
    }
    Ok(())
}

fn parse_artifact_expectation(
    value: Option<&Value>,
) -> Result<ArtifactExpectation, ProviderContractError> {
    parse(value, "artifactExpectation")
}

fn unique_chip_keys(chips: &[ChipProof], field: &'static str) -> Result<(), ProviderContractError> {
    let keys = chips
        .iter()
        .map(|chip| chip.chip_stable_key.as_str())
        .collect::<BTreeSet<_>>();
    (keys.len() == chips.len())
        .then_some(())
        .ok_or(ProviderContractError::Invalid(field))
}

fn requested_chip_pairs(chips: &[ChipProof]) -> BTreeSet<(&str, Option<&str>)> {
    chips
        .iter()
        .map(|chip| (chip.chip_stable_key.as_str(), chip.digest.as_deref()))
        .collect()
}

fn chip_pairs(chips: &[ClearedChip]) -> BTreeSet<(&str, Option<&str>)> {
    chips
        .iter()
        .map(|chip| (chip.chip_stable_key.as_str(), chip.digest.as_deref()))
        .collect()
}

fn optional_binding(
    value: Option<&Value>,
) -> Result<Option<PageBindingEcho>, ProviderContractError> {
    let binding = optional_parse::<PageBindingEcho>(value, "observedPageBinding")?;
    if let Some(binding) = &binding {
        binding
            .validate()
            .map_err(|_| ProviderContractError::Invalid("observedPageBinding"))?;
    }
    Ok(binding)
}

fn compare_binding(
    observed: Option<&PageBindingEcho>,
    expected: &PageBindingEcho,
) -> Result<(), ProviderContractError> {
    if observed.is_some_and(|observed| observed != expected) {
        return Err(ProviderContractError::BindingMismatch);
    }
    Ok(())
}

fn validate_expected_echo(
    expected: &SessionEcho,
    observed: &SessionEcho,
) -> Result<(), ProviderContractError> {
    let required = expected.page_binding == observed.page_binding
        && expected.session_id == observed.session_id
        && expected.conversation_url == observed.conversation_url
        && expected
            .request_id
            .as_ref()
            .is_none_or(|value| observed.request_id.as_ref() == Some(value))
        && expected
            .run_id
            .as_ref()
            .is_none_or(|value| observed.run_id.as_ref() == Some(value))
        && expected.session_binding_id == observed.session_binding_id
        && expected.page_binding_generation == observed.page_binding_generation
        && expected
            .visible_user_turn_id
            .as_ref()
            .is_none_or(|value| observed.visible_user_turn_id.as_ref() == Some(value))
        && expected
            .visible_assistant_turn_id
            .as_ref()
            .is_none_or(|value| observed.visible_assistant_turn_id.as_ref() == Some(value))
        && expected.active_turn == observed.active_turn
        && expected
            .terminal_answer_sha256
            .as_ref()
            .is_none_or(|value| observed.terminal_answer_sha256.as_ref() == Some(value));
    required
        .then_some(())
        .ok_or(ProviderContractError::BindingMismatch)
}

fn validate_failure_echo_rule(
    reason: Option<&str>,
    observed: Option<&SessionEcho>,
    expected: &SessionEcho,
) -> Result<(), ProviderContractError> {
    match reason {
        Some("session.url_rejected_root") => observed
            .is_none()
            .then_some(())
            .ok_or(ProviderContractError::Invalid("root observedEcho")),
        Some("session.url_rejected_mismatch") => {
            let observed = observed.ok_or(ProviderContractError::Invalid("observedEcho"))?;
            if validate_expected_echo(expected, observed).is_ok() {
                Err(ProviderContractError::Invalid("mismatch observedEcho"))
            } else {
                Ok(())
            }
        }
        _ => observed.map_or(Ok(()), |observed| {
            validate_expected_echo(expected, observed)
        }),
    }
}

fn is_session_failure(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some(
            "session.rebind_failed"
                | "session.pinned_slot_unavailable"
                | "session.content_unavailable"
                | "session.url_rejected_root"
                | "session.url_rejected_mismatch"
                | "session.missing"
                | "session.hydration_timeout"
                | "session.request_binding_missing"
                | "session.claim_conflict"
                | "session.provider_limit"
                | "session.login_required"
                | "session.subscription_required"
                | "session.schema_drift"
        )
    )
}

fn all_null(data: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| data.get(*key).is_some_and(Value::is_null))
}

fn validate_receipt_paths(
    operation: ProviderOperation,
    paths: &ReceiptRelPaths,
) -> Result<(), ProviderContractError> {
    let valid = match operation {
        ProviderOperation::SendClick => {
            paths.pre_click.is_some() && paths.post_click.is_some() && paths.reconcile.is_none()
        }
        ProviderOperation::SendReconcile => {
            paths.pre_click.is_some() && paths.post_click.is_none() && paths.reconcile.is_some()
        }
        _ => paths.pre_click.is_none() && paths.post_click.is_none() && paths.reconcile.is_none(),
    };
    valid
        .then_some(())
        .ok_or(ProviderContractError::Invalid("receiptRelPaths"))
}

fn request_keys(operation: ProviderOperation) -> &'static [&'static str] {
    match operation {
        ProviderOperation::Status => &["expectedSlotId", "probeAttempt"],
        ProviderOperation::CaptureRoot => {
            &["requestedModel", "requestedEffort", "rediscoveryAttempt"]
        }
        ProviderOperation::EnsureModel => &[
            "pageBinding",
            "requestedModel",
            "requestedEffort",
            "pickerOpenBudget",
            "stabilizationMs",
        ],
        ProviderOperation::UploadOnly => &[
            "pageBinding",
            "attachmentSet",
            "uploadAttemptId",
            "retryIndex",
        ],
        ProviderOperation::ClearUpload => &[
            "pageBinding",
            "uploadAttemptId",
            "clearAttemptId",
            "staleChips",
        ],
        ProviderOperation::SendClick => &[
            "pageBinding",
            "sendAttemptId",
            "uploadProof",
            "promptInput",
            "clickBudget",
        ],
        ProviderOperation::SendReconcile => &["pageBinding", "sendAttemptId", "preClickReceipt"],
        ProviderOperation::SessionRebind => &[
            "operationKind",
            "expectation",
            "navigationAttemptLimit",
            "hydrationDeadlineMs",
        ],
        ProviderOperation::Poll => &[
            "expected",
            "pollAttemptId",
            "pollTimeoutSeconds",
            "artifactExpectation",
        ],
        ProviderOperation::ArtifactDiscover => &[
            "expected",
            "artifactClaimId",
            "terminalAssistantTurnId",
            "expectation",
        ],
        ProviderOperation::ArtifactClickSave => &[
            "expected",
            "artifactClaimId",
            "terminalAssistantTurnId",
            "control",
            "baseline",
            "controlIndex",
            "hostSaveDirectory",
        ],
    }
}

fn response_keys(operation: ProviderOperation, ok: bool) -> &'static [&'static str] {
    match operation {
        ProviderOperation::Status => &[
            "healthStatus",
            "dockerStatus",
            "retryAfterMs",
            "modelLabel",
            "composerReady",
        ],
        ProviderOperation::CaptureRoot => &["rootBindingCandidate", "failureProof"],
        ProviderOperation::EnsureModel => &[
            "modelProof",
            "effortProof",
            "failureProof",
            "observedPageBinding",
        ],
        ProviderOperation::UploadOnly => &["uploadProof", "failureReason", "observedPageBinding"],
        ProviderOperation::ClearUpload if ok => {
            &["clearAttemptId", "clearedChips", "observedPageBinding"]
        }
        ProviderOperation::ClearUpload => &[
            "clearAttemptId",
            "failureReason",
            "attemptedChipKeys",
            "clearedChips",
            "observedPageBinding",
        ],
        ProviderOperation::SendClick | ProviderOperation::SendReconcile => &[
            "preClickReceipt",
            "terminalSendReceipt",
            "observedPageBinding",
        ],
        ProviderOperation::SessionRebind if ok => &[
            "expectation",
            "observedEcho",
            "pageBindingGeneration",
            "hydrationObservations",
            "terminalAnswer",
            "failureReason",
        ],
        ProviderOperation::SessionRebind => &[
            "expectation",
            "observedEcho",
            "pageBindingGeneration",
            "hydrationObservations",
            "failureReason",
        ],
        ProviderOperation::Poll => &[
            "expected",
            "observedEcho",
            "pollState",
            "answerSha256",
            "answerSizeBytes",
            "answerRelPath",
            "terminalAssistantTurnId",
            "bottomProof",
        ],
        ProviderOperation::ArtifactDiscover => &[
            "controls",
            "bottomProof",
            "zeroControlProof",
            "failureReason",
            "observedEcho",
        ],
        ProviderOperation::ArtifactClickSave => {
            &["downloadReceipt", "failureReason", "observedEcho"]
        }
    }
}

fn object(value: &Value) -> Result<&Map<String, Value>, ProviderContractError> {
    value
        .as_object()
        .ok_or(ProviderContractError::Invalid("operationData"))
}

fn text<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<&'a str, ProviderContractError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ProviderContractError::Invalid(field))
}

fn integer(
    object: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<u64, ProviderContractError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(ProviderContractError::Invalid(field))
}

fn optional_integer(
    object: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<Option<u64>, ProviderContractError> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value
                .as_u64()
                .ok_or(ProviderContractError::Invalid(field))?;
            if value > MAX_DURATION_MS {
                return Err(ProviderContractError::Invalid(field));
            }
            Ok(Some(value))
        }
        None => Err(ProviderContractError::Invalid(field)),
    }
}

fn boolean(
    object: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<bool, ProviderContractError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or(ProviderContractError::Invalid(field))
}

fn array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<&'a Vec<Value>, ProviderContractError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or(ProviderContractError::Invalid(field))
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    field: &'static str,
) -> Result<Option<String>, ProviderContractError> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && !value.contains('\0') => {
            Ok(Some(value.clone()))
        }
        _ => Err(ProviderContractError::Invalid(field)),
    }
}

fn id_value<'a, E>(
    object: &'a Map<String, Value>,
    key: &str,
    validate: fn(&str) -> Result<(), E>,
) -> Result<&'a str, ProviderContractError> {
    let value = text(object, key, "identifier")?;
    required_text(value, validate, "identifier")?;
    Ok(value)
}

fn required_text<E>(
    value: &str,
    validate: fn(&str) -> Result<(), E>,
    field: &'static str,
) -> Result<(), ProviderContractError> {
    validate(value).map_err(|_| ProviderContractError::Invalid(field))
}

fn optional_text<E>(
    value: &Option<String>,
    validate: fn(&str) -> Result<(), E>,
    field: &'static str,
) -> Result<(), ProviderContractError> {
    value
        .as_deref()
        .map(validate)
        .transpose()
        .map(|_| ())
        .map_err(|_| ProviderContractError::Invalid(field))
}

fn optional_parse<T: for<'de> Deserialize<'de>>(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<T>, ProviderContractError> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| ProviderContractError::Invalid(field)),
        None => Err(ProviderContractError::Invalid(field)),
    }
}

fn exact_keys(object: &Map<String, Value>, keys: &[&str]) -> Result<(), ProviderContractError> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    (actual == expected)
        .then_some(())
        .ok_or(ProviderContractError::Invalid("operationData fields"))
}

fn parse<T: for<'de> Deserialize<'de>>(
    value: Option<&Value>,
    field: &'static str,
) -> Result<T, ProviderContractError> {
    serde_json::from_value(
        value
            .cloned()
            .ok_or(ProviderContractError::Invalid(field))?,
    )
    .map_err(|_| ProviderContractError::Invalid(field))
}
