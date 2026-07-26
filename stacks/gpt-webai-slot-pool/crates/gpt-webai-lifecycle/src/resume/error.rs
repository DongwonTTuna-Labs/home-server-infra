use std::io;

use thiserror::Error;

use crate::confirmation::ConfirmationError;
use crate::locks::LockError;
use crate::provider_client::ProviderInvocationError;
use crate::provider_runner::ProviderRunnerError;
use crate::runtime::control::RuntimeControlError;
use crate::session_ops::runtime::SessionRuntimeError;
use crate::sessions::SessionRecordError;

#[derive(Debug, Error)]
pub(super) enum ResumeError {
    #[error("session record error: {0}")]
    Session(#[from] SessionRecordError),
    #[error("provider command failed: {0}")]
    ProviderCommand(#[from] ProviderRunnerError),
    #[error("provider invocation failed: {0}")]
    ProviderInvocation(#[from] ProviderInvocationError),
    #[error("runtime start failed: {0}")]
    RuntimeStart(#[from] SessionRuntimeError),
    #[error("runtime stop failed: {0}")]
    RuntimeRelease(#[from] RuntimeControlError),
    #[error("terminal confirmation failed: {0}")]
    Confirmation(#[from] ConfirmationError),
    #[error("answer artifact write failed: {0}")]
    AnswerArtifact(#[from] io::Error),
    #[error("slot lease is active; refusing resume")]
    ActiveLease,
    #[error("slot lock read failed: {0}")]
    LockRead(LockError),
    #[error("stale slot lock release failed: {0}")]
    LockRelease(LockError),
    #[error("session record write failed: {0}")]
    SessionWrite(SessionRecordError),
    #[error("provider returned session mismatch")]
    SessionMismatch,
}

pub(super) fn reason_for(error: &ResumeError) -> &'static str {
    match error {
        ResumeError::Session(SessionRecordError::Missing(_)) => "session.record_missing",
        ResumeError::Session(SessionRecordError::Collision(_))
        | ResumeError::Session(SessionRecordError::Invalid(_))
        | ResumeError::Session(SessionRecordError::InvalidConversationUrl(_))
        | ResumeError::Session(SessionRecordError::Json(_)) => "session.record_invalid",
        ResumeError::Session(SessionRecordError::Io(_)) => "session.record_read_failed",
        ResumeError::ProviderCommand(_) => "provider.command_failed",
        ResumeError::ProviderInvocation(_) => "provider.invocation_failed",
        ResumeError::RuntimeStart(_) => "runtime.start_failed",
        ResumeError::RuntimeRelease(_) => "runtime.stop_failed",
        ResumeError::Confirmation(_) => "answer.unconfirmed",
        ResumeError::AnswerArtifact(_) => "answer.artifact_write_failed",
        ResumeError::ActiveLease => "lock.active",
        ResumeError::LockRead(_) => "lock.read_failed",
        ResumeError::LockRelease(_) => "lock.release_failed",
        ResumeError::SessionWrite(_) => "session.write_failed",
        ResumeError::SessionMismatch => "session.url_mismatch",
    }
}
