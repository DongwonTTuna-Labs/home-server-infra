use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ids::{
    validate_binding_id, validate_browser_context_id, validate_byte_count, validate_claim_id,
    validate_cohort, validate_control_id, validate_conversation_url, validate_generation,
    validate_h256, validate_lease_id, validate_operation_id, validate_owner_id,
    validate_page_incarnation_id, validate_request_id, validate_root_id, validate_run_id,
    validate_safe_rel_path, validate_session_id, validate_slot_id, validate_target_id,
    validate_timestamp_ms, validate_turn_id,
};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BrowserContractError {
    #[error("invalid browser contract field: {0}")]
    Invalid(&'static str),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Pro,
    Xhigh,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Standard,
    High,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EvidenceMediaType {
    #[serde(rename = "application/json")]
    Json,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "application/octet-stream")]
    Binary,
    #[serde(rename = "text/markdown")]
    Markdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EvidenceRef {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: EvidenceMediaType,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ControlIdentity {
    pub bounding_box_hash: String,
    pub control_id: String,
    pub disabled: bool,
    pub dom_path_hash: String,
    pub label_hash: String,
    pub role: String,
    pub test_id_hash: Option<String>,
    pub visible: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RootBindingCandidate {
    pub browser_context_id: String,
    pub captured_at_ms: u64,
    pub composer_root_id: String,
    pub conversation_root_id: String,
    pub dom_mutation_generation: u16,
    pub effort_control: ControlIdentity,
    pub evidence_refs: Vec<EvidenceRef>,
    pub model_control: ControlIdentity,
    pub normalized_url: String,
    pub operation_id: String,
    pub page_incarnation_id: String,
    pub selector_margin: u32,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PageBindingEcho {
    pub binding_id: String,
    pub binding_generation: u16,
    pub slot_id: String,
    pub cohort: String,
    pub lease_id: String,
    pub lease_generation: u16,
    pub runtime_owner_id: String,
    pub runtime_owner_generation: u16,
    pub runtime_incarnation_id: String,
    pub browser_context_id: String,
    pub target_id: String,
    pub page_incarnation_id: String,
    pub root_binding_hash: String,
    pub dom_mutation_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionRebindExpectation {
    pub session_id: String,
    pub conversation_url: String,
    pub slot_id: String,
    pub cohort: String,
    pub session_operation_claim_id: Option<String>,
    pub lease_id: String,
    pub lease_generation: u16,
    pub runtime_owner_id: String,
    pub runtime_owner_generation: u16,
    pub runtime_incarnation_id: String,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub last_known_page_binding_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SessionEcho {
    #[serde(flatten)]
    pub page_binding: PageBindingEcho,
    pub session_id: String,
    pub conversation_url: String,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub session_binding_id: String,
    pub page_binding_generation: u16,
    pub visible_user_turn_id: Option<String>,
    pub visible_assistant_turn_id: Option<String>,
    pub active_turn: bool,
    pub terminal_answer_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelProof {
    pub requested: Model,
    pub observed: Model,
    pub verified: bool,
    pub control: ControlIdentity,
    pub selected_by: String,
    pub evidence_refs: Vec<EvidenceRef>,
    pub verified_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EffortProof {
    pub requested: Effort,
    pub observed: Effort,
    pub verified: bool,
    pub control: ControlIdentity,
    pub selected_by: String,
    pub evidence_refs: Vec<EvidenceRef>,
    pub verified_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FailureProof {
    pub reason: String,
    pub picker_opened: bool,
    pub requested_model_visible: bool,
    pub requested_effort_visible: bool,
    pub control_identity_stable: bool,
    pub evidence_refs: Vec<EvidenceRef>,
    pub failed_at_ms: u64,
}

impl EvidenceRef {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        valid(validate_safe_rel_path(&self.path), "evidence.path")?;
        valid(validate_h256(&self.sha256), "evidence.sha256")?;
        valid(validate_byte_count(self.size_bytes), "evidence.sizeBytes")
    }
}

impl ControlIdentity {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        valid(validate_h256(&self.bounding_box_hash), "boundingBoxHash")?;
        valid(validate_control_id(&self.control_id), "controlId")?;
        valid(validate_h256(&self.dom_path_hash), "domPathHash")?;
        valid(validate_h256(&self.label_hash), "labelHash")?;
        if let Some(value) = &self.test_id_hash {
            valid(validate_h256(value), "testIdHash")?;
        }
        if self.disabled
            || !self.visible
            || !matches!(
                self.role.as_str(),
                "button" | "combobox" | "menuitem" | "option"
            )
        {
            return Err(BrowserContractError::Invalid("control state/role"));
        }
        Ok(())
    }
}

impl RootBindingCandidate {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        valid(
            validate_browser_context_id(&self.browser_context_id),
            "browserContextId",
        )?;
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        valid(validate_root_id(&self.composer_root_id), "composerRootId")?;
        valid(
            validate_root_id(&self.conversation_root_id),
            "conversationRootId",
        )?;
        self.effort_control.validate()?;
        self.model_control.validate()?;
        evidence(&self.evidence_refs, 1, 4)?;
        valid(validate_operation_id(&self.operation_id), "operationId")?;
        valid(
            validate_page_incarnation_id(&self.page_incarnation_id),
            "pageIncarnationId",
        )?;
        valid(validate_target_id(&self.target_id), "targetId")?;
        if !(50..=100_000).contains(&self.selector_margin)
            || (self.normalized_url != "https://chatgpt.com/"
                && !self.normalized_url.starts_with("https://chatgpt.com/c/"))
        {
            return Err(BrowserContractError::Invalid(
                "selectorMargin/normalizedUrl",
            ));
        }
        if let Some(session_id) = self.normalized_url.strip_prefix("https://chatgpt.com/c/") {
            valid(
                validate_conversation_url(&self.normalized_url, session_id),
                "normalizedUrl",
            )?;
        }
        Ok(())
    }
}

impl PageBindingEcho {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        valid(validate_binding_id(&self.binding_id), "bindingId")?;
        valid(
            validate_generation(self.binding_generation),
            "bindingGeneration",
        )?;
        valid(validate_slot_id(&self.slot_id), "slotId")?;
        valid(validate_cohort(&self.cohort), "cohort")?;
        valid(validate_lease_id(&self.lease_id), "leaseId")?;
        valid(
            validate_generation(self.lease_generation),
            "leaseGeneration",
        )?;
        valid(validate_owner_id(&self.runtime_owner_id), "runtimeOwnerId")?;
        valid(
            validate_generation(self.runtime_owner_generation),
            "runtimeOwnerGeneration",
        )?;
        valid(
            super::ids::validate_runtime_incarnation_id(&self.runtime_incarnation_id),
            "runtimeIncarnationId",
        )?;
        valid(
            validate_browser_context_id(&self.browser_context_id),
            "browserContextId",
        )?;
        valid(validate_target_id(&self.target_id), "targetId")?;
        valid(
            validate_page_incarnation_id(&self.page_incarnation_id),
            "pageIncarnationId",
        )?;
        valid(validate_h256(&self.root_binding_hash), "rootBindingHash")?;
        let expected =
            super::ids::derive_page_binding_id(&self.page_incarnation_id, &self.root_binding_hash)
                .map_err(|_| BrowserContractError::Invalid("bindingId"))?;
        if self.binding_id != expected {
            return Err(BrowserContractError::Invalid("bindingId"));
        }
        Ok(())
    }
}

impl SessionRebindExpectation {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        valid(validate_session_id(&self.session_id), "sessionId")?;
        valid(
            validate_conversation_url(&self.conversation_url, &self.session_id),
            "conversationUrl",
        )?;
        valid(validate_slot_id(&self.slot_id), "slotId")?;
        valid(validate_cohort(&self.cohort), "cohort")?;
        optional(
            &self.session_operation_claim_id,
            validate_claim_id,
            "sessionOperationClaimId",
        )?;
        valid(validate_lease_id(&self.lease_id), "leaseId")?;
        valid(
            validate_generation(self.lease_generation),
            "leaseGeneration",
        )?;
        valid(validate_owner_id(&self.runtime_owner_id), "runtimeOwnerId")?;
        valid(
            validate_generation(self.runtime_owner_generation),
            "runtimeOwnerGeneration",
        )?;
        valid(
            super::ids::validate_runtime_incarnation_id(&self.runtime_incarnation_id),
            "runtimeIncarnationId",
        )?;
        optional(&self.request_id, validate_request_id, "requestId")?;
        optional(&self.run_id, validate_run_id, "runId")
    }
}

impl SessionEcho {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        self.page_binding.validate()?;
        valid(validate_session_id(&self.session_id), "sessionId")?;
        valid(
            validate_conversation_url(&self.conversation_url, &self.session_id),
            "conversationUrl",
        )?;
        optional(&self.request_id, validate_request_id, "requestId")?;
        optional(&self.run_id, validate_run_id, "runId")?;
        valid(
            validate_binding_id(&self.session_binding_id),
            "sessionBindingId",
        )?;
        let expected = super::ids::derive_session_binding_id(
            &self.session_id,
            &self.page_binding.slot_id,
            &self.page_binding.cohort,
        )
        .map_err(|_| BrowserContractError::Invalid("sessionBindingId"))?;
        if self.session_binding_id != expected {
            return Err(BrowserContractError::Invalid("sessionBindingId"));
        }
        valid(
            validate_generation(self.page_binding_generation),
            "pageBindingGeneration",
        )?;
        optional(
            &self.visible_user_turn_id,
            validate_turn_id,
            "visibleUserTurnId",
        )?;
        optional(
            &self.visible_assistant_turn_id,
            validate_turn_id,
            "visibleAssistantTurnId",
        )?;
        optional(
            &self.terminal_answer_sha256,
            validate_h256,
            "terminalAnswerSha256",
        )
    }
}

impl ModelProof {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        self.control.validate()?;
        evidence(&self.evidence_refs, 1, 4)?;
        valid(validate_timestamp_ms(self.verified_at_ms), "verifiedAtMs")?;
        if !self.verified
            || self.observed != self.requested
            || !matches!(self.selected_by.as_str(), "already_exact" | "picker")
        {
            return Err(BrowserContractError::Invalid("ModelProof"));
        }
        Ok(())
    }
}

impl EffortProof {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        self.control.validate()?;
        evidence(&self.evidence_refs, 1, 4)?;
        valid(validate_timestamp_ms(self.verified_at_ms), "verifiedAtMs")?;
        if !self.verified
            || self.observed != self.requested
            || !matches!(self.selected_by.as_str(), "already_exact" | "picker")
        {
            return Err(BrowserContractError::Invalid("EffortProof"));
        }
        Ok(())
    }
}

impl FailureProof {
    pub fn validate(&self) -> Result<(), BrowserContractError> {
        evidence(&self.evidence_refs, 1, 4)?;
        valid(validate_timestamp_ms(self.failed_at_ms), "failedAtMs")?;
        if !matches!(
            self.reason.as_str(),
            "picker.model_absent"
                | "picker.effort_absent"
                | "picker.control_drift"
                | "picker.selection_timeout"
                | "picker.reverify_mismatch"
                | "capture.ambiguous"
        ) {
            return Err(BrowserContractError::Invalid("FailureProof.reason"));
        }
        Ok(())
    }
}

pub fn validate_model_tuple(model: &Model, effort: &Effort) -> Result<(), BrowserContractError> {
    matches!(
        (model, effort),
        (Model::Pro, Effort::Standard) | (Model::Xhigh, Effort::High)
    )
    .then_some(())
    .ok_or(BrowserContractError::Invalid("model/effort tuple"))
}

fn evidence(values: &[EvidenceRef], min: usize, max: usize) -> Result<(), BrowserContractError> {
    if !(min..=max).contains(&values.len()) {
        return Err(BrowserContractError::Invalid("evidenceRefs"));
    }
    values.iter().try_for_each(EvidenceRef::validate)
}

fn optional(
    value: &Option<String>,
    check: fn(&str) -> Result<(), super::ids::IdError>,
    field: &'static str,
) -> Result<(), BrowserContractError> {
    value
        .as_deref()
        .map(check)
        .transpose()
        .map(|_| ())
        .map_err(|_| BrowserContractError::Invalid(field))
}

fn valid<T, E>(result: Result<T, E>, field: &'static str) -> Result<(), BrowserContractError> {
    result
        .map(|_| ())
        .map_err(|_| BrowserContractError::Invalid(field))
}
