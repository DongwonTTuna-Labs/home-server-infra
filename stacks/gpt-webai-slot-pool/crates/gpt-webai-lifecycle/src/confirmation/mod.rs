use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::contracts::ids::validate_turn_id;
use crate::provider_client::{validate_provider_envelope, ProviderContractError};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendStartConfirmation {
    pub session_id: String,
    pub conversation_url: String,
    pub target_id: String,
    pub active_turn: bool,
    pub user_turn_id: String,
    pub assistant_turn_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAnswerConfirmation {
    pub session_id: String,
    pub conversation_url: String,
    pub target_id: String,
    pub answer_text: String,
    pub answer_text_len: usize,
    pub answer_text_sha256: String,
    pub assistant_turn_text_sha256: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ConfirmationError {
    #[error("provider contract invalid: {0}")]
    ProviderContract(String),
    #[error("provider status is not sent: {0}")]
    NotSent(String),
    #[error("provider status is not done: {0}")]
    NotDone(String),
    #[error("sent envelope missing required evidence: {0}")]
    MissingEvidence(&'static str),
    #[error("conversation url does not match session id")]
    UrlSessionMismatch,
    #[error("sent envelope does not prove both server-assigned turn identities")]
    NoRealTurnEvidence,
    #[error("terminal answer is empty")]
    EmptyAnswer,
}

pub fn confirm_send_started(value: &Value) -> Result<SendStartConfirmation, ConfirmationError> {
    let summary = validate_provider_envelope(value).map_err(provider_contract_error)?;
    if summary.status != "sent" {
        return Err(ConfirmationError::NotSent(summary.status));
    }
    let session_id = summary
        .session_id
        .ok_or(ConfirmationError::MissingEvidence("sessionId"))?;
    let conversation_url = summary
        .conversation_url
        .ok_or(ConfirmationError::MissingEvidence("conversationUrl"))?;
    if !conversation_url_matches_session(&conversation_url, &session_id) {
        return Err(ConfirmationError::UrlSessionMismatch);
    }
    let object = value
        .as_object()
        .ok_or(ConfirmationError::MissingEvidence("object"))?;
    let target_id = object
        .get("targetId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ConfirmationError::MissingEvidence("targetId"))?
        .to_string();
    let turn_evidence = object
        .get("turnEvidence")
        .and_then(Value::as_object)
        .ok_or(ConfirmationError::MissingEvidence("turnEvidence"))?;
    let active_turn = bool_field(turn_evidence.get("activeTurn"));
    let user_turn_id = required_turn_id(turn_evidence, "userTurnId")?;
    let assistant_turn_id = required_turn_id(turn_evidence, "assistantTurnId")?;
    Ok(SendStartConfirmation {
        session_id,
        conversation_url,
        target_id,
        active_turn,
        user_turn_id,
        assistant_turn_id,
    })
}

pub fn confirm_terminal_answer(
    value: &Value,
) -> Result<TerminalAnswerConfirmation, ConfirmationError> {
    confirm_terminal_answer_for_statuses(value, &["done"])
}

pub fn confirm_terminal_answer_for_statuses(
    value: &Value,
    accepted_statuses: &[&str],
) -> Result<TerminalAnswerConfirmation, ConfirmationError> {
    let summary = validate_provider_envelope(value).map_err(provider_contract_error)?;
    if !accepted_statuses.contains(&summary.status.as_str()) {
        return Err(ConfirmationError::NotDone(summary.status));
    }
    let session_id = summary
        .session_id
        .ok_or(ConfirmationError::MissingEvidence("sessionId"))?;
    let conversation_url = summary
        .conversation_url
        .ok_or(ConfirmationError::MissingEvidence("conversationUrl"))?;
    if !conversation_url_matches_session(&conversation_url, &session_id) {
        return Err(ConfirmationError::UrlSessionMismatch);
    }
    let object = value
        .as_object()
        .ok_or(ConfirmationError::MissingEvidence("object"))?;
    let target_id = object
        .get("targetId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ConfirmationError::MissingEvidence("targetId"))?
        .to_string();
    let answer_text = object
        .get("answerText")
        .and_then(Value::as_str)
        .ok_or(ConfirmationError::MissingEvidence("answerText"))?
        .trim();
    if answer_text.is_empty() {
        return Err(ConfirmationError::EmptyAnswer);
    }
    let answer_text = answer_text.to_string();
    let assistant_turn = object
        .get("assistantTurn")
        .and_then(Value::as_object)
        .ok_or(ConfirmationError::MissingEvidence("assistantTurn"))?;
    let assistant_turn_text_sha256 = assistant_turn
        .get("textSha256")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 64)
        .ok_or(ConfirmationError::MissingEvidence(
            "assistantTurn.textSha256",
        ))?
        .to_string();
    Ok(TerminalAnswerConfirmation {
        session_id,
        conversation_url,
        target_id,
        answer_text_len: answer_text.len(),
        answer_text_sha256: sha256_text(&answer_text),
        answer_text,
        assistant_turn_text_sha256,
    })
}

fn provider_contract_error(error: ProviderContractError) -> ConfirmationError {
    ConfirmationError::ProviderContract(error.to_string())
}

fn bool_field(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

fn required_turn_id(
    evidence: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<String, ConfirmationError> {
    let value = evidence
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| validate_turn_id(value).is_ok())
        .ok_or(ConfirmationError::NoRealTurnEvidence)?;
    Ok(value.to_string())
}

fn conversation_url_matches_session(url: &str, session_id: &str) -> bool {
    url == format!("https://chatgpt.com/c/{session_id}")
}

fn sha256_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}
