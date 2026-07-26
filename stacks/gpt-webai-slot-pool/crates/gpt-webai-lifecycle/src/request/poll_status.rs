use crate::confirmation::confirm_terminal_answer_for_statuses;
use crate::provider_client::ProviderInvocationResult;
use crate::provider_runner::ProviderCommand;
use crate::sessions::SessionRecord;
use crate::slots::AllocationDecision;

use super::artifacts::{finish_terminal_artifact_failure, TerminalRunContext};
use super::failure_output::failed_after_runtime_start;
use super::input::RequestRunInput;
use super::output::RequestRunOutput;
use super::poll::write_nonterminal_poll_artifacts;
use super::runtime::RuntimeStart;
use super::session::mark_released;

pub(crate) struct NonDonePollContext<'a> {
    pub(crate) input: &'a RequestRunInput,
    pub(crate) decision: &'a AllocationDecision,
    pub(crate) provider_command: &'a ProviderCommand,
    pub(crate) session: &'a SessionRecord,
    pub(crate) poll: &'a ProviderInvocationResult,
    pub(crate) send_status: Option<String>,
    pub(crate) poll_status: Option<String>,
    pub(crate) runtime_start: &'a RuntimeStart,
}

pub(crate) fn handle_non_done_poll_status(
    context: NonDonePollContext<'_>,
) -> Option<RequestRunOutput> {
    if context.poll.summary.status == "done" {
        return None;
    }
    if let Err(error) = write_nonterminal_poll_artifacts(
        context.input,
        context.provider_command,
        context.session,
        &context.poll.value,
    ) {
        mark_released(
            context.input,
            context.session.clone(),
            Some("poll.raw_write_failed".to_string()),
        );
        return Some(
            failed_after_runtime_start(
                context.input,
                context.decision,
                "poll.raw_write_failed",
                error.to_string(),
                context.runtime_start,
            )
            .with_send_status(context.send_status.clone())
            .with_poll_status(context.poll_status.clone())
            .with_session_start(
                &context.session.session_id,
                &context.session.conversation_url,
            ),
        );
    }
    if context.poll.summary.status == "provider_limit" {
        mark_released(
            context.input,
            context.session.clone(),
            Some("provider.limit".to_string()),
        );
        return Some(
            failed_after_runtime_start(
                context.input,
                context.decision,
                "provider.limit",
                "provider poll reported provider limit".to_string(),
                context.runtime_start,
            )
            .with_send_status(context.send_status.clone())
            .with_poll_status(context.poll_status.clone())
            .with_session_start(
                &context.session.session_id,
                &context.session.conversation_url,
            ),
        );
    }
    if is_scroll_bottom_unverified_status(&context.poll.summary.status) {
        mark_released(
            context.input,
            context.session.clone(),
            Some("scroll.bottom_unverified".to_string()),
        );
        return Some(
            failed_after_runtime_start(
                context.input,
                context.decision,
                "scroll.bottom_unverified",
                "provider poll reported unverified bottom-scroll proof".to_string(),
                context.runtime_start,
            )
            .with_send_status(context.send_status.clone())
            .with_poll_status(context.poll_status.clone())
            .with_session_start(
                &context.session.session_id,
                &context.session.conversation_url,
            ),
        );
    }
    if is_artifact_failure_status(&context.poll.summary.status) {
        return Some(handle_artifact_failure_status(context));
    }
    None
}

fn handle_artifact_failure_status(context: NonDonePollContext<'_>) -> RequestRunOutput {
    let terminal = match confirm_terminal_answer_for_statuses(
        &context.poll.value,
        &[
            "artifact.controls_absent",
            "artifact.download_timeout",
            "artifact.recovery_failed",
        ],
    ) {
        Ok(terminal) => terminal,
        Err(error) => {
            mark_released(
                context.input,
                context.session.clone(),
                Some("answer.unconfirmed".to_string()),
            );
            return failed_after_runtime_start(
                context.input,
                context.decision,
                "answer.unconfirmed",
                error.to_string(),
                context.runtime_start,
            )
            .with_send_status(context.send_status.clone())
            .with_poll_status(context.poll_status.clone())
            .with_session_start(
                &context.session.session_id,
                &context.session.conversation_url,
            );
        }
    };
    let status = context.poll.summary.status.clone();
    finish_terminal_artifact_failure(
        TerminalRunContext {
            input: context.input,
            decision: context.decision,
            provider_command: context.provider_command,
            session: context.session.clone(),
            terminal,
            poll_summary: context.poll.summary.clone(),
            send_status: context.send_status.clone(),
            poll_status: context.poll_status.clone(),
            runtime_started: context.runtime_start.runtime_started,
            runtime_owned: context.runtime_start.runtime_owned,
        },
        &status,
        format!("provider poll reported terminal artifact failure: {status}"),
    )
}

fn is_artifact_failure_status(status: &str) -> bool {
    matches!(
        status,
        "artifact.controls_absent" | "artifact.download_timeout" | "artifact.recovery_failed"
    )
}

fn is_scroll_bottom_unverified_status(status: &str) -> bool {
    matches!(
        status,
        "scroll.bottom_unverified" | "session.running_unverified"
    )
}
