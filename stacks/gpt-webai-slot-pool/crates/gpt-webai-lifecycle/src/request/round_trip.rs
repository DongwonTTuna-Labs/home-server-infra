use crate::confirmation::confirm_terminal_answer;
use crate::provider_runner::ProviderCommandContext;
use crate::runtime::RuntimeProbe;
use crate::sessions::{self, NewSessionRecord};
use crate::slots::AllocationDecision;

use super::artifacts::{finish_terminal_run, TerminalRunContext};
use super::failure_output::failed_after_runtime_start;
use super::input::RequestRunInput;
use super::output::{failed_output, RequestRunOutput};
use super::poll_status::{handle_non_done_poll_status, NonDonePollContext};
use super::provider::provider_poll;
use super::runtime::ensure_runtime_started;
use super::selection::persist_allocation_cursors;
use super::send_start::send_and_confirm_start;
use super::session::mark_released;
use super::visual_gate::run_pre_send_visual_gate;
use super::wait_gate::{run_pre_poll_wait_gate, RequestWaitGateInput};

pub(crate) fn run_with_acquired_lease(
    input: &RequestRunInput,
    decision: &AllocationDecision,
    runtime: &dyn RuntimeProbe,
) -> RequestRunOutput {
    if let Err(error) = persist_allocation_cursors(input, decision) {
        return failed_output(
            input,
            Some(decision),
            "cursor.write_failed",
            error.to_string(),
        );
    }

    let runtime_start = match ensure_runtime_started(input, runtime, decision) {
        Ok(start) => start,
        Err(error) => {
            return failed_output(
                input,
                Some(decision),
                "runtime.start_failed",
                error.to_string(),
            );
        }
    };

    let provider_command = match input.provider_execution.command(ProviderCommandContext {
        config: &input.config,
        slot_id: &decision.slot_id.0,
        run_id: &input.run_id,
    }) {
        Ok(command) => command,
        Err(error) => {
            return failed_after_runtime_start(
                input,
                decision,
                "provider.command_failed",
                error.to_string(),
                &runtime_start,
            );
        }
    };

    if input.pre_send_visual_gate {
        if let Err(error) = run_pre_send_visual_gate(input, &provider_command) {
            return failed_after_runtime_start(
                input,
                decision,
                "visual_gate.failed",
                error.to_string(),
                &runtime_start,
            );
        }
    }

    let send = match send_and_confirm_start(input, decision, &provider_command, &runtime_start) {
        Ok(send) => send,
        Err(output) => return *output,
    };
    let send_status = send.send_status;
    let start = send.start;

    let session = match sessions::new_session_record(NewSessionRecord {
        request_id: Some(input.request_id.clone()),
        run_id: Some(input.run_id.clone()),
        session_id: start.session_id.clone(),
        conversation_url: start.conversation_url.clone(),
        slot_id: decision.slot_id.0.clone(),
        cohort: crate::allocator::cohort_of(&decision.slot_id.0)
            .expect("allocated slot has a canonical cohort")
            .to_string(),
        page_binding_generation: 1,
    }) {
        Ok(record) => record,
        Err(error) => {
            return failed_after_runtime_start(
                input,
                decision,
                "session.record_failed",
                error.to_string(),
                &runtime_start,
            )
            .with_send_status(send_status);
        }
    };
    if let Err(error) = sessions::write_session_record(&input.config.state_root, &session) {
        return failed_after_runtime_start(
            input,
            decision,
            "session.write_failed",
            error.to_string(),
            &runtime_start,
        )
        .with_send_status(send_status)
        .with_session(&session);
    }

    if input.pre_poll_wait_gate {
        let gate = run_pre_poll_wait_gate(RequestWaitGateInput {
            request: input,
            command: &provider_command,
            session_id: &start.session_id,
            conversation_url: &start.conversation_url,
            real_turn_evidence: true,
        });
        if let Err(error) = gate {
            mark_released(
                input,
                session,
                Some("session.running_unverified".to_string()),
            );
            return failed_after_runtime_start(
                input,
                decision,
                "session.running_unverified",
                error.to_string(),
                &runtime_start,
            )
            .with_send_status(send_status)
            .with_session_start(&start.session_id, &start.conversation_url);
        }
    }

    let poll = match provider_poll(input, &provider_command, &start.session_id) {
        Ok(poll) => poll,
        Err(error) => {
            mark_released(input, session, Some("provider.poll_failed".to_string()));
            return failed_after_runtime_start(
                input,
                decision,
                "provider.poll_failed",
                error.to_string(),
                &runtime_start,
            )
            .with_send_status(send_status)
            .with_session_start(&start.session_id, &start.conversation_url);
        }
    };
    let poll_status = Some(poll.summary.status.clone());
    if let Some(output) = handle_non_done_poll_status(NonDonePollContext {
        input,
        decision,
        provider_command: &provider_command,
        session: &session,
        poll: &poll,
        send_status: send_status.clone(),
        poll_status: poll_status.clone(),
        runtime_start: &runtime_start,
    }) {
        return output;
    }

    let terminal = match confirm_terminal_answer(&poll.value) {
        Ok(terminal) => terminal,
        Err(error) => {
            mark_released(input, session, Some("answer.unconfirmed".to_string()));
            return failed_after_runtime_start(
                input,
                decision,
                "answer.unconfirmed",
                error.to_string(),
                &runtime_start,
            )
            .with_send_status(send_status)
            .with_poll_status(poll_status)
            .with_session_start(&start.session_id, &start.conversation_url);
        }
    };
    finish_terminal_run(TerminalRunContext {
        input,
        decision,
        provider_command: &provider_command,
        session,
        terminal,
        poll_summary: poll.summary,
        send_status,
        poll_status,
        runtime_started: runtime_start.runtime_started,
        runtime_owned: runtime_start.runtime_owned,
    })
}
