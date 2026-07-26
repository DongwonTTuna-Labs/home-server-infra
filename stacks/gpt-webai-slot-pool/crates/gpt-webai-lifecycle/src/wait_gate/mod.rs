use serde_json::Value;
use thiserror::Error;

use crate::provider_client::{validate_provider_envelope, ProviderContractError};
use crate::scroll_proof::{diagnostics_saved, scroll_bottom_verified};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WaitGateError {
    #[error("provider contract invalid: {0}")]
    ProviderContract(String),
    #[error("capture status is not captured: {0}")]
    NotCaptured(String),
    #[error("capture did not save screenshot and DOM")]
    CaptureMissing,
    #[error("capture bottom-scroll proof was not verified")]
    BottomScrollUnverified,
    #[error("capture conversation URL does not match session")]
    UrlSessionMismatch,
    #[error("send start had no real turn evidence")]
    NoRealTurnEvidence,
}

#[derive(Clone, Copy, Debug)]
pub struct PrePollWaitGateEvidence<'a> {
    pub capture_value: &'a Value,
    pub session_id: &'a str,
    pub conversation_url: &'a str,
    pub real_turn_evidence: bool,
}

pub fn confirm_pre_poll_wait_gate(
    evidence: PrePollWaitGateEvidence<'_>,
) -> Result<(), WaitGateError> {
    if !evidence.real_turn_evidence {
        return Err(WaitGateError::NoRealTurnEvidence);
    }
    let capture =
        validate_provider_envelope(evidence.capture_value).map_err(provider_contract_error)?;
    if matches!(
        capture.status.as_str(),
        "scroll.bottom_unverified" | "session.running_unverified"
    ) {
        return Err(WaitGateError::BottomScrollUnverified);
    }
    if capture.status != "captured" {
        return Err(WaitGateError::NotCaptured(capture.status));
    }
    if !diagnostics_saved(evidence.capture_value) {
        return Err(WaitGateError::CaptureMissing);
    }
    if !scroll_bottom_verified(evidence.capture_value) {
        return Err(WaitGateError::BottomScrollUnverified);
    }
    let expected_url = format!("https://chatgpt.com/c/{}", evidence.session_id);
    if evidence.conversation_url != expected_url
        || capture.conversation_url.as_deref() != Some(evidence.conversation_url)
    {
        return Err(WaitGateError::UrlSessionMismatch);
    }
    Ok(())
}

fn provider_contract_error(error: ProviderContractError) -> WaitGateError {
    WaitGateError::ProviderContract(error.to_string())
}
