use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde_json::json;
use thiserror::Error;

use crate::config::{now_ms, SupervisorConfig};
use crate::contracts::browser::{EvidenceRef, SessionEcho};
use crate::contracts::cli::ArtifactClaimSummary;
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::h256;
use crate::provider_runner::ProviderExecution;
use crate::sessions::SessionRecord;

use super::artifacts::{
    recover_artifacts, ArtifactPipelineError, ArtifactPipelineInput, ArtifactPipelineOutput,
};
use super::journal::{NewEvent, SessionJournal, SessionJournalError};
use super::provider::{PollResponseData, ProviderLimits};

#[derive(Debug, Error)]
pub enum TerminalPipelineError {
    #[error("terminal pipeline journal failed: {0}")]
    Journal(#[from] SessionJournalError),
    #[error("terminal pipeline artifact recovery failed: {0}")]
    Artifact(#[from] ArtifactPipelineError),
    #[error("terminal pipeline io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal pipeline contract failed: {0}")]
    Contract(&'static str),
}

pub struct PollTerminalInput<'a> {
    pub config: &'a SupervisorConfig,
    pub journal: &'a mut SessionJournal,
    pub provider_execution: &'a ProviderExecution,
    pub provider_limits: ProviderLimits,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub request_id: &'a str,
    pub run_id: &'a str,
    pub record: &'a SessionRecord,
    pub expected: &'a SessionEcho,
    pub hydrated: &'a EventEnvelope,
    pub poll_started: &'a EventEnvelope,
    pub poll_attempt_id: &'a str,
    pub poll_receipt: &'a EvidenceRef,
    pub poll_data: &'a PollResponseData,
    pub artifacts_host_dir: &'a Path,
    pub artifact_expectation: &'a str,
}

pub struct TerminalPipelineOutput {
    pub source_event: EventEnvelope,
    pub result: TerminalResult,
    pub answer_path: Option<String>,
    pub answer_sha256: Option<String>,
    pub answer_size_bytes: Option<u64>,
    pub answer_text: Option<String>,
    pub artifact_claims: Vec<ArtifactClaimSummary>,
    pub receipt_ids: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalResult {
    Success,
    OptionalZero,
    ArtifactFailed,
}

pub fn persist_poll_terminal(
    input: PollTerminalInput<'_>,
) -> Result<TerminalPipelineOutput, TerminalPipelineError> {
    validate_terminal_observation(&input)?;
    let answer_sha256 = input
        .poll_data
        .answer_sha256
        .as_deref()
        .ok_or(TerminalPipelineError::Contract("answerSha256"))?;
    let answer_size_bytes = input
        .poll_data
        .answer_size_bytes
        .ok_or(TerminalPipelineError::Contract("answerSizeBytes"))?;
    let answer_rel_path = input
        .poll_data
        .answer_rel_path
        .as_deref()
        .ok_or(TerminalPipelineError::Contract("answerRelPath"))?;
    let terminal_turn_id = input
        .poll_data
        .terminal_assistant_turn_id
        .as_deref()
        .ok_or(TerminalPipelineError::Contract("terminalAssistantTurnId"))?;
    let source = input.artifacts_host_dir.join(answer_rel_path);
    let answer_path = format!(
        "answers/{}/{}.answer.md",
        input.request_key, input.poll_attempt_id
    );
    let target = input.config.state_root.join(&answer_path);
    let bytes = copy_verified_answer(
        &input.config.state_root,
        &source,
        &target,
        answer_sha256,
        answer_size_bytes,
    )?;
    let terminal_at = now_ms();
    let answer_terminal = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: input.request_id.to_string(),
        event_type: EventType::AnswerTerminal,
        payload: json!({
            "requestId":input.request_id,"pollAttemptId":input.poll_attempt_id,
            "sessionId":input.record.session_id,"answerPath":answer_path,
            "answerSha256":answer_sha256,"answerSizeBytes":answer_size_bytes,
            "terminalAssistantTurnId":terminal_turn_id,"pollReceipt":input.poll_receipt,
            "terminalAtMs":terminal_at
        }),
        predecessor_event_id: Some(input.poll_started.event_id.clone()),
        source_event_ids: vec![input.hydrated.event_id.clone()],
        created_at_ms: terminal_at,
    })?;
    let artifacts = recover_artifacts(ArtifactPipelineInput {
        config: input.config,
        journal: input.journal,
        provider_execution: input.provider_execution,
        provider_limits: input.provider_limits,
        operation_id: input.operation_id,
        request_key: input.request_key,
        request_id: Some(input.request_id),
        run_id: Some(input.run_id),
        record: input.record,
        expected: input
            .poll_data
            .observed_echo
            .as_ref()
            .ok_or(TerminalPipelineError::Contract("observedEcho"))?,
        source_event: &answer_terminal,
        terminal_assistant_turn_id: terminal_turn_id,
        expectation: input.artifact_expectation,
    })?;
    persist_after_artifact(input, answer_terminal, artifacts, answer_path, bytes)
}

fn persist_after_artifact(
    input: PollTerminalInput<'_>,
    answer_terminal: EventEnvelope,
    artifacts: ArtifactPipelineOutput,
    answer_path: String,
    bytes: Vec<u8>,
) -> Result<TerminalPipelineOutput, TerminalPipelineError> {
    let persisted_at = now_ms();
    let persisted = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: input.request_id.to_string(),
        event_type: EventType::TerminalPersisted,
        payload: json!({
            "requestId":input.request_id,"answerTerminalEventId":answer_terminal.event_id,
            "artifactClaimEventIds":[artifacts.terminal_event.event_id],
            "outputPath":answer_path,"persistedAtMs":persisted_at
        }),
        predecessor_event_id: Some(answer_terminal.event_id.clone()),
        source_event_ids: vec![
            answer_terminal.event_id.clone(),
            artifacts.terminal_event.event_id.clone(),
        ],
        created_at_ms: persisted_at,
    })?;
    let summary = artifacts.summary;
    if let Some(reason) = artifacts.failure_reason {
        return Ok(TerminalPipelineOutput {
            source_event: artifacts.terminal_event,
            result: TerminalResult::ArtifactFailed,
            answer_path: None,
            answer_sha256: None,
            answer_size_bytes: None,
            answer_text: None,
            artifact_claims: vec![summary],
            receipt_ids: artifacts.receipt_ids,
            reason: Some(reason),
        });
    }
    let published_at = now_ms();
    let published = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: input.request_id.to_string(),
        event_type: EventType::OutputPublished,
        payload: json!({
            "requestId":input.request_id,"outputPath":answer_path,
            "outputSha256":h256(&bytes),"publishedAtMs":published_at
        }),
        predecessor_event_id: Some(persisted.event_id.clone()),
        source_event_ids: vec![persisted.event_id],
        created_at_ms: published_at,
    })?;
    let text = if !bytes.is_empty() && bytes.len() <= 65_536 {
        String::from_utf8(bytes.clone()).ok()
    } else {
        None
    };
    Ok(TerminalPipelineOutput {
        source_event: published,
        result: if artifacts.optional_zero {
            TerminalResult::OptionalZero
        } else {
            TerminalResult::Success
        },
        answer_path: Some(answer_path),
        answer_sha256: input.poll_data.answer_sha256.clone(),
        answer_size_bytes: input.poll_data.answer_size_bytes,
        answer_text: text,
        artifact_claims: vec![summary],
        receipt_ids: artifacts.receipt_ids,
        reason: None,
    })
}

fn validate_terminal_observation(
    input: &PollTerminalInput<'_>,
) -> Result<(), TerminalPipelineError> {
    let data = input.poll_data;
    if data.poll_state != "terminal"
        || data.expected != *input.expected
        || data.observed_echo.as_ref().is_none_or(|echo| {
            echo.active_turn
                || echo.session_id != input.record.session_id
                || echo.conversation_url != input.record.conversation_url
                || echo.terminal_answer_sha256 != data.answer_sha256
                || echo.visible_assistant_turn_id != data.terminal_assistant_turn_id
        })
        || data.bottom_proof.is_some()
    {
        return Err(TerminalPipelineError::Contract("terminal poll observation"));
    }
    Ok(())
}

fn copy_verified_answer(
    state_root: &Path,
    source: &Path,
    target: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<Vec<u8>, std::io::Error> {
    let mut source_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(source)?;
    let metadata = source_file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != expected_size {
        return Err(std::io::Error::other("answer source metadata mismatch"));
    }
    let mut bytes = Vec::new();
    source_file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected_size || h256(&bytes) != expected_sha256 {
        return Err(std::io::Error::other("answer source digest mismatch"));
    }
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("answer target parent missing"))?;
    crate::provider_runner::create_private_directory(state_root, parent)?;
    let mut target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)?;
    target_file.write_all(&bytes)?;
    target_file.sync_all()?;
    File::open(parent)?.sync_all()?;
    let persisted = fs::read(target)?;
    if persisted != bytes {
        return Err(std::io::Error::other("answer target verification failed"));
    }
    Ok(bytes)
}
