pub mod state;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::browser::{EvidenceRef, PageBindingEcho};
use crate::contracts::ids::{
    validate_conversation_url, validate_h256, validate_operation_id, validate_session_id,
    validate_timestamp_ms, validate_turn_id,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendReceiptKind {
    PreClick,
    PostClick,
    ReconciledTurnStart,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SendReceipt {
    pub kind: SendReceiptKind,
    pub send_attempt_id: String,
    pub page_binding: PageBindingEcho,
    pub prompt_sha256: String,
    pub physical_click_count: u8,
    pub user_turn_id: Option<String>,
    pub assistant_turn_id: Option<String>,
    pub session_id: Option<String>,
    pub conversation_url: Option<String>,
    pub captured_at_ms: u64,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnStart {
    pub session_id: String,
    pub conversation_url: String,
    pub user_turn_id: String,
    pub assistant_turn_id: String,
    pub physical_click_count: u8,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SendReconcileError {
    #[error("invalid send receipt field: {0}")]
    Invalid(&'static str),
    #[error("illegal send transition")]
    IllegalTransition,
}

impl SendReceipt {
    pub fn validate(&self) -> Result<(), SendReconcileError> {
        valid(
            validate_operation_id(&self.send_attempt_id),
            "sendAttemptId",
        )?;
        self.page_binding
            .validate()
            .map_err(|_| SendReconcileError::Invalid("pageBinding"))?;
        valid(validate_h256(&self.prompt_sha256), "promptSha256")?;
        valid(validate_timestamp_ms(self.captured_at_ms), "capturedAtMs")?;
        validate_evidence(&self.evidence_refs)?;
        match self.kind {
            SendReceiptKind::PreClick => {
                if self.physical_click_count != 0 || !self.turn_fields_all_null() {
                    return Err(SendReconcileError::Invalid("pre_click nullability"));
                }
            }
            SendReceiptKind::PostClick => self.validate_terminal(1)?,
            SendReceiptKind::ReconciledTurnStart => self.validate_terminal(0)?,
        }
        Ok(())
    }

    pub fn terminal_turn_start(&self) -> Result<TurnStart, SendReconcileError> {
        self.validate()?;
        if self.kind == SendReceiptKind::PreClick {
            return Err(SendReconcileError::Invalid("terminal receipt kind"));
        }
        Ok(TurnStart {
            session_id: self.session_id.clone().expect("validated"),
            conversation_url: self.conversation_url.clone().expect("validated"),
            user_turn_id: self.user_turn_id.clone().expect("validated"),
            assistant_turn_id: self.assistant_turn_id.clone().expect("validated"),
            physical_click_count: self.physical_click_count,
        })
    }

    fn validate_terminal(&self, click_count: u8) -> Result<(), SendReconcileError> {
        let (Some(user), Some(assistant), Some(session), Some(url)) = (
            self.user_turn_id.as_deref(),
            self.assistant_turn_id.as_deref(),
            self.session_id.as_deref(),
            self.conversation_url.as_deref(),
        ) else {
            return Err(SendReconcileError::Invalid("terminal nullability"));
        };
        if self.physical_click_count != click_count {
            return Err(SendReconcileError::Invalid("physicalClickCount"));
        }
        valid(validate_turn_id(user), "userTurnId")?;
        valid(validate_turn_id(assistant), "assistantTurnId")?;
        valid(validate_session_id(session), "sessionId")?;
        valid(validate_conversation_url(url, session), "conversationUrl")
    }

    fn turn_fields_all_null(&self) -> bool {
        self.user_turn_id.is_none()
            && self.assistant_turn_id.is_none()
            && self.session_id.is_none()
            && self.conversation_url.is_none()
    }
}

pub fn validate_receipt_pair(
    pre_click: &SendReceipt,
    terminal: &SendReceipt,
    expected_binding: &PageBindingEcho,
) -> Result<TurnStart, SendReconcileError> {
    pre_click.validate()?;
    terminal.validate()?;
    if pre_click.kind != SendReceiptKind::PreClick
        || terminal.kind == SendReceiptKind::PreClick
        || pre_click.send_attempt_id != terminal.send_attempt_id
        || pre_click.prompt_sha256 != terminal.prompt_sha256
        || &pre_click.page_binding != expected_binding
        || &terminal.page_binding != expected_binding
        || terminal.captured_at_ms < pre_click.captured_at_ms
    {
        return Err(SendReconcileError::Invalid("receipt pair binding"));
    }
    terminal.terminal_turn_start()
}

fn validate_evidence(values: &[EvidenceRef]) -> Result<(), SendReconcileError> {
    if !(1..=4).contains(&values.len()) || values.iter().any(|item| item.validate().is_err()) {
        return Err(SendReconcileError::Invalid("evidenceRefs"));
    }
    let paths = values
        .iter()
        .map(|item| item.path.as_str())
        .collect::<BTreeSet<_>>();
    (paths.len() == values.len())
        .then_some(())
        .ok_or(SendReconcileError::Invalid("duplicate evidenceRefs"))
}

fn valid<T, E>(result: Result<T, E>, field: &'static str) -> Result<(), SendReconcileError> {
    result
        .map(|_| ())
        .map_err(|_| SendReconcileError::Invalid(field))
}
