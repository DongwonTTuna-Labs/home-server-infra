use std::collections::BTreeSet;

use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RetainedReceiptToken {
    AnswerPollTerminal,
    AnswerResumeTerminal,
    ArtifactDownload,
    ArtifactPollTerminal,
    ArtifactResumeTerminal,
    CaptureRoot,
    CaptureSession,
    ClaimZero,
    FailureInvocation,
    FailureProvider,
    PollFailure,
    PollProgress,
    PriorAnswerTerminal,
    SendPostClick,
    SendPreClick,
    SendReconciledTurnStart,
    SendStart,
    SessionResume,
    SessionShow,
    Status,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReceiptTokenError {
    #[error("unknown retained receipt token: {0}")]
    Unknown(String),
    #[error("retained receipt sequence contains an empty or duplicate token")]
    InvalidSequence,
    #[error("retained receipt sequence exceeds the closed three-receipt bound")]
    TooMany,
    #[error("prior retained receipts are not an exact prefix of the total sequence")]
    PriorNotPrefix,
    #[error("retained receipt count differs from the parsed sequence")]
    CountMismatch,
}

impl RetainedReceiptToken {
    pub fn parse(value: &str) -> Result<Self, ReceiptTokenError> {
        match value {
            "receipt.answer.poll_terminal" => Ok(Self::AnswerPollTerminal),
            "receipt.answer.resume_terminal" => Ok(Self::AnswerResumeTerminal),
            "receipt.artifact.download" => Ok(Self::ArtifactDownload),
            "receipt.artifact.poll_terminal" => Ok(Self::ArtifactPollTerminal),
            "receipt.artifact.resume_terminal" => Ok(Self::ArtifactResumeTerminal),
            "receipt.capture.root" => Ok(Self::CaptureRoot),
            "receipt.capture.session" => Ok(Self::CaptureSession),
            "receipt.claim.zero" => Ok(Self::ClaimZero),
            "receipt.failure.invocation" => Ok(Self::FailureInvocation),
            "receipt.failure.provider" => Ok(Self::FailureProvider),
            "receipt.poll.failure" => Ok(Self::PollFailure),
            "receipt.poll.progress" => Ok(Self::PollProgress),
            "receipt.prior.answer_terminal" => Ok(Self::PriorAnswerTerminal),
            "receipt.send.post_click" => Ok(Self::SendPostClick),
            "receipt.send.pre_click" => Ok(Self::SendPreClick),
            "receipt.send.reconciled_turn_start" => Ok(Self::SendReconciledTurnStart),
            "receipt.send.start" => Ok(Self::SendStart),
            "receipt.session.resume" => Ok(Self::SessionResume),
            "receipt.session.show" => Ok(Self::SessionShow),
            "receipt.status" => Ok(Self::Status),
            other => Err(ReceiptTokenError::Unknown(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnswerPollTerminal => "receipt.answer.poll_terminal",
            Self::AnswerResumeTerminal => "receipt.answer.resume_terminal",
            Self::ArtifactDownload => "receipt.artifact.download",
            Self::ArtifactPollTerminal => "receipt.artifact.poll_terminal",
            Self::ArtifactResumeTerminal => "receipt.artifact.resume_terminal",
            Self::CaptureRoot => "receipt.capture.root",
            Self::CaptureSession => "receipt.capture.session",
            Self::ClaimZero => "receipt.claim.zero",
            Self::FailureInvocation => "receipt.failure.invocation",
            Self::FailureProvider => "receipt.failure.provider",
            Self::PollFailure => "receipt.poll.failure",
            Self::PollProgress => "receipt.poll.progress",
            Self::PriorAnswerTerminal => "receipt.prior.answer_terminal",
            Self::SendPostClick => "receipt.send.post_click",
            Self::SendPreClick => "receipt.send.pre_click",
            Self::SendReconciledTurnStart => "receipt.send.reconciled_turn_start",
            Self::SendStart => "receipt.send.start",
            Self::SessionResume => "receipt.session.resume",
            Self::SessionShow => "receipt.session.show",
            Self::Status => "receipt.status",
        }
    }
}

pub fn parse_retained_receipts(cell: &str) -> Result<Vec<RetainedReceiptToken>, ReceiptTokenError> {
    if cell == "none" {
        return Ok(Vec::new());
    }
    if cell.is_empty() {
        return Err(ReceiptTokenError::InvalidSequence);
    }
    let tokens = cell
        .split(',')
        .map(RetainedReceiptToken::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if tokens.len() > 3 {
        return Err(ReceiptTokenError::TooMany);
    }
    if tokens.iter().copied().collect::<BTreeSet<_>>().len() != tokens.len() {
        return Err(ReceiptTokenError::InvalidSequence);
    }
    Ok(tokens)
}

pub fn validate_receipt_prefix(
    prior: &[RetainedReceiptToken],
    total: &[RetainedReceiptToken],
    expected_count: u8,
) -> Result<(), ReceiptTokenError> {
    if total.len() > 3 {
        return Err(ReceiptTokenError::TooMany);
    }
    if total.len() != usize::from(expected_count) {
        return Err(ReceiptTokenError::CountMismatch);
    }
    if !total.starts_with(prior) {
        return Err(ReceiptTokenError::PriorNotPrefix);
    }
    Ok(())
}
