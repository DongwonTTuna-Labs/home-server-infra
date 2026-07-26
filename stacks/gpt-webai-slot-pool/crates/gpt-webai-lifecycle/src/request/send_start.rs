use crate::confirmation::{confirm_send_started, SendStartConfirmation};
use crate::provider_runner::ProviderCommand;
use crate::slots::AllocationDecision;

use super::failure_output::failed_after_runtime_start;
use super::input::RequestRunInput;
use super::output::RequestRunOutput;
use super::provider::provider_send;
use super::runtime::RuntimeStart;
use super::send_start_recovery::{recover_send_start_from_artifacts, SendStartRecovery};

pub(crate) struct ConfirmedSendStart {
    pub(crate) send_status: Option<String>,
    pub(crate) start: SendStartConfirmation,
}

pub(crate) fn send_and_confirm_start(
    input: &RequestRunInput,
    decision: &AllocationDecision,
    provider_command: &ProviderCommand,
    runtime_start: &RuntimeStart,
) -> Result<ConfirmedSendStart, Box<RequestRunOutput>> {
    let (send_status, start) = match provider_send(input, provider_command) {
        Ok(send) => {
            let send_status = Some(send.summary.status.clone());
            let start = match confirm_send_started(&send.value) {
                Ok(start) => start,
                Err(error) => {
                    if send_start_status_recoverable(&send.summary.status) {
                        match recover_send_start_from_artifacts(provider_command) {
                            Some(SendStartRecovery::Confirmed(recovered)) => recovered.start,
                            Some(SendStartRecovery::Unconfirmed(recovered)) => {
                                let message = format!(
                                    "{}; {} at {}",
                                    error,
                                    recovered.message,
                                    recovered.source_path.display()
                                );
                                return Err(Box::new(
                                    failed_after_runtime_start(
                                        input,
                                        decision,
                                        "session.start_unconfirmed",
                                        message,
                                        runtime_start,
                                    )
                                    .with_send_status(send_status)
                                    .with_session_evidence(
                                        recovered.session_id,
                                        recovered.conversation_url,
                                    ),
                                ));
                            }
                            None => {
                                let reason =
                                    send_start_confirmation_failure_reason(&send.summary.status);
                                return Err(Box::new(
                                    failed_after_runtime_start(
                                        input,
                                        decision,
                                        reason,
                                        error.to_string(),
                                        runtime_start,
                                    )
                                    .with_send_status(send_status)
                                    .with_session_evidence(
                                        send.summary.session_id,
                                        send.summary.conversation_url,
                                    ),
                                ));
                            }
                        }
                    } else {
                        let reason = send_start_confirmation_failure_reason(&send.summary.status);
                        return Err(Box::new(
                            failed_after_runtime_start(
                                input,
                                decision,
                                reason,
                                error.to_string(),
                                runtime_start,
                            )
                            .with_send_status(send_status)
                            .with_session_evidence(
                                send.summary.session_id,
                                send.summary.conversation_url,
                            ),
                        ));
                    }
                }
            };
            (send_status, start)
        }
        Err(error) => match recover_send_start_from_artifacts(provider_command) {
            Some(SendStartRecovery::Confirmed(recovered)) => {
                let _message = format!(
                    "{}; {} at {}",
                    error,
                    recovered.message,
                    recovered.source_path.display()
                );
                (Some("sent".to_string()), recovered.start)
            }
            Some(SendStartRecovery::Unconfirmed(recovered)) => {
                let message = format!(
                    "{}; {} at {}",
                    error,
                    recovered.message,
                    recovered.source_path.display()
                );
                return Err(Box::new(
                    failed_after_runtime_start(
                        input,
                        decision,
                        "session.start_unconfirmed",
                        message,
                        runtime_start,
                    )
                    .with_send_status(Some("session.start_unconfirmed".to_string()))
                    .with_session_evidence(recovered.session_id, recovered.conversation_url),
                ));
            }
            None => {
                return Err(Box::new(failed_after_runtime_start(
                    input,
                    decision,
                    "provider.send_failed",
                    error.to_string(),
                    runtime_start,
                )));
            }
        },
    };
    Ok(ConfirmedSendStart { send_status, start })
}

fn send_start_status_recoverable(status: &str) -> bool {
    matches!(status, "sent" | "session.start_unconfirmed")
}

fn send_start_confirmation_failure_reason(status: &str) -> &str {
    match status {
        "sent" | "session.start_unconfirmed" => "session.start_unconfirmed",
        "model.selection_mismatch" => "model.selection_mismatch",
        _ => "send.confirmation_failed",
    }
}
