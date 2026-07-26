use std::path::Path;

use crate::contracts::browser::PageBindingEcho;
use crate::contracts::events::EventEnvelope;
use crate::provider_runner::R13ProviderCommandContext;
use crate::session_ops::journal::SessionJournal;

use super::input::RequestRunInput;
use super::r13_assets::StagedFreshAssets;
use super::r13_events::{
    append_binding, append_send_armed, append_send_clicked, append_send_failed,
    append_send_reconciled, append_send_uncertain, BindingEvents,
};
use super::r13_provider::{
    identity, invoke, send_click_request, send_reconcile_request, FreshProviderLimits, SendData,
};
use super::r13_types::{child_operation_id, FreshRunError};
use super::r13_upload::UploadStage;

pub struct SendStage {
    pub binding: BindingEvents,
    pub receipt_ids: Vec<String>,
}

pub struct SendFailure {
    pub source_event: EventEnvelope,
    pub reason: String,
    pub receipt_ids: Vec<String>,
    pub result_kind: &'static str,
}

pub enum SendExecution {
    Ready(Box<SendStage>),
    Failed(Box<SendFailure>),
}

#[allow(clippy::too_many_arguments)]
pub fn send_and_bind(
    input: &RequestRunInput,
    operation_id: &str,
    journal: &mut SessionJournal,
    root_event: &EventEnvelope,
    slot_id: &str,
    cohort: &str,
    page: &PageBindingEcho,
    upload: &UploadStage,
    assets: &StagedFreshAssets,
) -> Result<SendExecution, FreshRunError> {
    let request_key = format!("r-{}", input.request_id);
    let send_id = child_operation_id(operation_id, "send")?;
    let send_command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &send_id,
        })?;
    let reconcile_id = child_operation_id(operation_id, "reconcile")?;
    let reconcile_command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &reconcile_id,
        })?;
    let pre_path = state_relative(
        &input.config.state_root,
        &send_command
            .paths
            .operation_host_dir
            .join("send.pre-click.receipt.json"),
    )?;
    let post_path = state_relative(
        &input.config.state_root,
        &send_command
            .paths
            .operation_host_dir
            .join("send.post-click.receipt.json"),
    )?;
    let reconcile_path = state_relative(
        &input.config.state_root,
        &reconcile_command
            .paths
            .operation_host_dir
            .join("send.reconcile.receipt.json"),
    )?;
    let armed = append_send_armed(
        journal,
        &upload.completed_event,
        &input.request_id,
        &send_id,
        &upload.upload_attempt_id,
        page,
        &assets.prompt_sha256,
        [&pre_path, &post_path, &reconcile_path],
    )?;
    let limits = FreshProviderLimits {
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    };
    let sent = match invoke::<SendData>(
        &input.config.state_root,
        &send_command,
        &send_click_request(
            identity(cohort, &send_id, &input.request_id, &input.run_id, slot_id),
            page,
            &send_id,
            &upload.proof,
            &assets.prompt_input,
        ),
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return Ok(SendExecution::Failed(Box::new(SendFailure {
                source_event: append_send_failed(
                    journal,
                    &armed,
                    &input.request_id,
                    &send_id,
                    "contract.invalid_provider_envelope",
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: Vec::new(),
                result_kind: "run.send_failed",
            })));
        }
    };
    if sent.data.observed_page_binding.as_ref() != Some(page)
        || sent.data.pre_click_receipt.page_binding != *page
        || sent.data.pre_click_receipt.send_attempt_id != send_id
    {
        return Ok(SendExecution::Failed(Box::new(SendFailure {
            source_event: append_send_failed(
                journal,
                &armed,
                &input.request_id,
                &send_id,
                "binding.mismatch",
                Some(&sent.receipt),
            )?,
            reason: "binding.mismatch".to_string(),
            receipt_ids: sent.receipt_ids,
            result_kind: "run.send_failed",
        })));
    }

    let (terminal_event, terminal_receipt, receipt_ids) = if let Some(terminal) =
        sent.data.terminal_send_receipt.as_ref().filter(|_| sent.ok)
    {
        let event = append_send_clicked(
            journal,
            &armed,
            &input.request_id,
            &sent.data.pre_click_receipt,
            terminal,
        )?;
        (event, terminal.clone(), sent.receipt_ids)
    } else {
        let reconciled = match invoke::<SendData>(
            &input.config.state_root,
            &reconcile_command,
            &send_reconcile_request(
                identity(
                    cohort,
                    &reconcile_id,
                    &input.request_id,
                    &input.run_id,
                    slot_id,
                ),
                page,
                &send_id,
                &sent.data.pre_click_receipt,
            ),
            limits,
        ) {
            Ok(result) => result,
            Err(_) => {
                return Ok(SendExecution::Failed(Box::new(SendFailure {
                    source_event: append_send_uncertain(
                        journal,
                        &armed,
                        &input.request_id,
                        &send_id,
                    )?,
                    reason: "send.turn_not_proven".to_string(),
                    receipt_ids: sent.receipt_ids,
                    result_kind: "run.send_uncertain",
                })));
            }
        };
        let Some(terminal) = reconciled.data.terminal_send_receipt else {
            let mut receipt_ids = sent.receipt_ids;
            receipt_ids.extend(reconciled.receipt_ids);
            return Ok(SendExecution::Failed(Box::new(SendFailure {
                source_event: append_send_uncertain(journal, &armed, &input.request_id, &send_id)?,
                reason: "send.turn_not_proven".to_string(),
                receipt_ids,
                result_kind: "run.send_uncertain",
            })));
        };
        if !reconciled.ok
            || reconciled.data.observed_page_binding.as_ref() != Some(page)
            || reconciled.data.pre_click_receipt != sent.data.pre_click_receipt
        {
            let mut receipt_ids = sent.receipt_ids;
            receipt_ids.extend(reconciled.receipt_ids);
            return Ok(SendExecution::Failed(Box::new(SendFailure {
                source_event: append_send_uncertain(journal, &armed, &input.request_id, &send_id)?,
                reason: "send.turn_not_proven".to_string(),
                receipt_ids,
                result_kind: "run.send_uncertain",
            })));
        }
        let event = append_send_reconciled(
            journal,
            &armed,
            &input.request_id,
            &sent.data.pre_click_receipt,
            &terminal,
        )?;
        let mut ids = sent.receipt_ids;
        ids.extend(reconciled.receipt_ids);
        (event, terminal, ids)
    };
    let binding = append_binding(
        journal,
        &terminal_event,
        root_event,
        &input.request_id,
        slot_id,
        cohort,
        page,
        &sent.data.pre_click_receipt,
        &terminal_receipt,
    )?;
    Ok(SendExecution::Ready(Box::new(SendStage {
        binding,
        receipt_ids,
    })))
}

fn state_relative(state_root: &Path, path: &Path) -> Result<String, FreshRunError> {
    path.strip_prefix(state_root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or(FreshRunError::Contract("receipt path"))
}
