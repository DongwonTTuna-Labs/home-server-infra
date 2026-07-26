use serde_json::Value;
use thiserror::Error;

use crate::provider_client::{validate_provider_envelope, ProviderContractError};
use crate::scroll_proof::{diagnostics_saved, scroll_bottom_verified};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum VisualGateError {
    #[error("provider contract invalid: {0}")]
    ProviderContract(String),
    #[error("provider status is not ready: {0}")]
    NotReady(String),
    #[error("provider capture did not save screenshot and DOM")]
    CaptureMissing,
    #[error("provider status diagnostics did not save screenshot and DOM")]
    StatusDiagnosticsMissing,
    #[error("provider bottom-scroll proof was not verified for {0}")]
    BottomScrollUnverified(&'static str),
    #[error("provider readiness signal failed: {0}")]
    ReadinessSignal(&'static str),
}

pub fn confirm_pre_send_visual_gate(
    status_value: &Value,
    capture_value: &Value,
) -> Result<(), VisualGateError> {
    let status = validate_provider_envelope(status_value).map_err(provider_contract_error)?;
    if status.status != "ready" {
        return Err(VisualGateError::NotReady(status.status));
    }
    if !diagnostics_saved(status_value) {
        return Err(VisualGateError::StatusDiagnosticsMissing);
    }
    require_ready_signals(status_value)?;

    let capture = validate_provider_envelope(capture_value).map_err(provider_contract_error)?;
    if matches!(
        capture.status.as_str(),
        "scroll.bottom_unverified" | "session.running_unverified"
    ) {
        return Err(VisualGateError::BottomScrollUnverified("capture"));
    }
    if capture.status != "captured" || !diagnostics_saved(capture_value) {
        return Err(VisualGateError::CaptureMissing);
    }

    if pre_send_root_composer_without_turns(status_value)
        && pre_send_root_composer_without_turns(capture_value)
    {
        return Ok(());
    }

    if !scroll_bottom_verified(status_value) {
        return Err(VisualGateError::BottomScrollUnverified("status"));
    }
    if !scroll_bottom_verified(capture_value) {
        return Err(VisualGateError::BottomScrollUnverified("capture"));
    }
    Ok(())
}

fn provider_contract_error(error: ProviderContractError) -> VisualGateError {
    VisualGateError::ProviderContract(error.to_string())
}

fn require_ready_signals(value: &Value) -> Result<(), VisualGateError> {
    let signals = value
        .get("diagnostics")
        .and_then(|diagnostics| diagnostics.get("readinessSignals"))
        .and_then(Value::as_object)
        .ok_or(VisualGateError::ReadinessSignal("readinessSignals"))?;
    if bool_signal(signals.get("login")) {
        return Err(VisualGateError::ReadinessSignal("login"));
    }
    if bool_signal(signals.get("limit")) {
        return Err(VisualGateError::ReadinessSignal("limit"));
    }
    if !bool_signal(signals.get("pro")) {
        return Err(VisualGateError::ReadinessSignal("pro"));
    }
    if !bool_signal(signals.get("composer")) {
        return Err(VisualGateError::ReadinessSignal("composer"));
    }
    Ok(())
}

fn pre_send_root_composer_without_turns(value: &Value) -> bool {
    let Some(diagnostics) = value.get("diagnostics").and_then(Value::as_object) else {
        return false;
    };
    let best_url = diagnostics
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .or_else(|| string_field(value, "url"))
        .or_else(|| string_field(value, "conversationUrl"))
        .unwrap_or_default();
    root_chatgpt_url(best_url)
        && envelope_root_url_or_empty(value, "url")
        && envelope_root_url_or_empty(value, "conversationUrl")
        && session_id_empty(value, diagnostics)
        && no_assistant_turns(diagnostics)
        && stop_controls_count(diagnostics) == Some(0)
}

fn root_chatgpt_url(url: &str) -> bool {
    let trimmed = url.trim_end_matches('/');
    matches!(
        trimmed,
        "https://chatgpt.com" | "https://www.chatgpt.com" | "http://chatgpt.com"
    ) || (trimmed.starts_with("https://chatgpt.com/?")
        || trimmed.starts_with("https://www.chatgpt.com/?")
        || trimmed.starts_with("http://chatgpt.com/?"))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn envelope_root_url_or_empty(value: &Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|url| url.is_empty() || root_chatgpt_url(url))
        .unwrap_or(true)
}

fn session_id_empty(value: &Value, diagnostics: &serde_json::Map<String, Value>) -> bool {
    let envelope_session = value
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let diagnostics_session = diagnostics
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    envelope_session.is_empty() && diagnostics_session.is_empty()
}

fn no_assistant_turns(diagnostics: &serde_json::Map<String, Value>) -> bool {
    let selector_turns = diagnostics
        .get("selectorInventory")
        .and_then(|inventory| inventory.get("assistantTurns"))
        .and_then(Value::as_u64);
    if selector_turns != Some(0) {
        return false;
    }
    diagnostics
        .get("assistantTurns")
        .and_then(Value::as_array)
        .map(|turns| turns.is_empty())
        .unwrap_or(true)
}

fn stop_controls_count(diagnostics: &serde_json::Map<String, Value>) -> Option<u64> {
    diagnostics
        .get("readinessSignals")
        .and_then(|signals| signals.get("stopControls"))
        .and_then(Value::as_u64)
}

fn bool_signal(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}
