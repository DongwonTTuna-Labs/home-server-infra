use std::io;

use crate::answer_artifacts::{
    write_answer_artifacts as persist_answer_artifacts, AnswerArtifactContext,
};
use crate::confirmation::TerminalAnswerConfirmation;
use crate::provider_client::ProviderEnvelopeSummary;
use crate::provider_runner::ProviderCommand;
use crate::sessions::SessionRecord;
use crate::slots::AllocationDecision;

use super::artifact_recovery::{
    artifact_failure_reason, recover_artifacts, ArtifactRecovery, ArtifactRecoveryError,
};
use super::input::RequestRunInput;
use super::output::{failed_output, schema, RequestRunOutput};
use super::session::mark_released;

pub(crate) struct TerminalRunContext<'a> {
    pub input: &'a RequestRunInput,
    pub decision: &'a AllocationDecision,
    pub provider_command: &'a ProviderCommand,
    pub session: SessionRecord,
    pub terminal: TerminalAnswerConfirmation,
    pub poll_summary: ProviderEnvelopeSummary,
    pub send_status: Option<String>,
    pub poll_status: Option<String>,
    pub runtime_started: bool,
    pub runtime_owned: bool,
}

pub(crate) fn finish_terminal_artifact_failure(
    context: TerminalRunContext<'_>,
    failure_status: &str,
    message: String,
) -> RequestRunOutput {
    if let Err(error) = write_answer_artifacts(&context) {
        return answer_artifact_failed_output(context, error);
    }
    mark_released(
        context.input,
        context.session,
        Some(failure_status.to_string()),
    );
    failed_output(
        context.input,
        Some(context.decision),
        failure_status,
        message,
    )
    .with_send_status(context.send_status)
    .with_poll_status(context.poll_status)
    .with_download_status(Some(failure_status.to_string()))
    .with_artifact_counts(
        context.poll_summary.artifacts,
        context.poll_summary.artifact_candidates,
    )
    .with_answer_text_len(context.terminal.answer_text_len)
    .with_runtime_started(context.runtime_started)
    .with_runtime_owned(context.runtime_owned)
    .with_session_start(
        &context.terminal.session_id,
        &context.terminal.conversation_url,
    )
}

pub(crate) fn finish_terminal_run(context: TerminalRunContext<'_>) -> RequestRunOutput {
    if let Err(error) = write_answer_artifacts(&context) {
        return answer_artifact_failed_output(context, error);
    }
    match recover_artifacts(&context) {
        Ok(recovery) => success_output(context, recovery),
        Err((error, recovery)) => artifact_failed_output(context, error, recovery),
    }
}

fn write_answer_artifacts(context: &TerminalRunContext<'_>) -> io::Result<()> {
    persist_answer_artifacts(AnswerArtifactContext {
        config: &context.input.config,
        path_mode: &context.provider_command.path_mode,
        request_id: &context.input.request_id,
        run_id: &context.input.run_id,
        terminal: &context.terminal,
    })
}

fn success_output(context: TerminalRunContext<'_>, recovery: ArtifactRecovery) -> RequestRunOutput {
    mark_released(
        context.input,
        context.session,
        Some("answer.done".to_string()),
    );
    RequestRunOutput {
        schema: schema(),
        ok: true,
        status: "done".to_string(),
        reason: None,
        request_id: context.input.request_id.clone(),
        run_id: context.input.run_id.clone(),
        slot_id: Some(context.decision.slot_id.0.clone()),
        account_group: Some(context.decision.allocated_group.0.clone()),
        preferred_group: Some(context.decision.preferred_group.0.clone()),
        session_id: Some(context.terminal.session_id),
        conversation_url: Some(context.terminal.conversation_url),
        lock_acquired: true,
        lock_released: false,
        runtime_started: context.runtime_started,
        runtime_owned: context.runtime_owned,
        runtime_stopped: false,
        slot_state_written: false,
        send_status: context.send_status,
        poll_status: context.poll_status,
        download_status: recovery.download_status,
        send_attempts: 0,
        send_retry_delays_ms: Vec::new(),
        provider_limit_retry_delays_ms: Vec::new(),
        artifacts: recovery.artifacts,
        artifact_candidates: recovery.artifact_candidates,
        answer_text_len: Some(context.terminal.answer_text_len),
        message: "provider send/poll round trip completed".to_string(),
    }
}

fn answer_artifact_failed_output(
    context: TerminalRunContext<'_>,
    error: io::Error,
) -> RequestRunOutput {
    mark_released(
        context.input,
        context.session,
        Some("answer.artifact_write_failed".to_string()),
    );
    failed_output(
        context.input,
        Some(context.decision),
        "answer.artifact_write_failed",
        error.to_string(),
    )
    .with_send_status(context.send_status)
    .with_poll_status(context.poll_status)
    .with_answer_text_len(context.terminal.answer_text_len)
    .with_runtime_started(context.runtime_started)
    .with_runtime_owned(context.runtime_owned)
    .with_session_start(
        &context.terminal.session_id,
        &context.terminal.conversation_url,
    )
}

fn artifact_failed_output(
    context: TerminalRunContext<'_>,
    error: ArtifactRecoveryError,
    recovery: ArtifactRecovery,
) -> RequestRunOutput {
    let reason = artifact_failure_reason(&error);
    mark_released(context.input, context.session, Some(reason.to_string()));
    failed_output(
        context.input,
        Some(context.decision),
        reason,
        error.to_string(),
    )
    .with_send_status(context.send_status)
    .with_poll_status(context.poll_status)
    .with_download_status(recovery.download_status)
    .with_artifact_counts(recovery.artifacts, recovery.artifact_candidates)
    .with_answer_text_len(context.terminal.answer_text_len)
    .with_runtime_started(context.runtime_started)
    .with_runtime_owned(context.runtime_owned)
    .with_session_start(
        &context.terminal.session_id,
        &context.terminal.conversation_url,
    )
}
