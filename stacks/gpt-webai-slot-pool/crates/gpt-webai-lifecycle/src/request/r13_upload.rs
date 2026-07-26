use crate::contracts::browser::PageBindingEcho;
use crate::contracts::events::EventEnvelope;
use crate::provider_runner::R13ProviderCommandContext;
use crate::session_ops::journal::SessionJournal;
use crate::uploads::UploadProof;

use super::input::RequestRunInput;
use super::r13_assets::StagedFreshAssets;
use super::r13_events::{
    append_upload_cleared, append_upload_completed, append_upload_failed, append_upload_mismatch,
    append_upload_started,
};
use super::r13_provider::{
    clear_request, identity, invoke, upload_request, ClearData, FreshProviderLimits, UploadData,
};
use super::r13_types::{child_operation_id, FreshRunError};

pub struct UploadStage {
    pub completed_event: EventEnvelope,
    pub proof: UploadProof,
    pub upload_attempt_id: String,
    pub receipt_ids: Vec<String>,
}

pub struct UploadFailure {
    pub source_event: EventEnvelope,
    pub reason: String,
    pub receipt_ids: Vec<String>,
}

pub enum UploadExecution {
    Ready(UploadStage),
    Failed(UploadFailure),
}

#[allow(clippy::too_many_arguments)]
pub fn run_upload(
    input: &RequestRunInput,
    operation_id: &str,
    journal: &mut SessionJournal,
    materialized: &EventEnvelope,
    slot_id: &str,
    cohort: &str,
    page: &PageBindingEcho,
    assets: &StagedFreshAssets,
    limits: FreshProviderLimits,
) -> Result<UploadExecution, FreshRunError> {
    let request_key = format!("r-{}", input.request_id);
    let first_id = child_operation_id(operation_id, "upload0")?;
    let first_started = append_upload_started(
        journal,
        materialized,
        &input.request_id,
        &first_id,
        0,
        &assets.attachment_set.set_sha256,
        &page.root_binding_hash,
    )?;
    let first = match invoke_upload(
        input,
        &request_key,
        slot_id,
        cohort,
        page,
        assets,
        &first_id,
        0,
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return Ok(UploadExecution::Failed(UploadFailure {
                source_event: append_upload_failed(
                    journal,
                    &first_started,
                    &input.request_id,
                    &first_id,
                    0,
                    "contract.invalid_provider_envelope",
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: Vec::new(),
            }));
        }
    };
    let first_proof = match require_proof(&first, page, &first_id, 0) {
        Ok(proof) => proof,
        Err(reason) => {
            let source_event = append_upload_failed(
                journal,
                &first_started,
                &input.request_id,
                &first_id,
                0,
                reason,
                Some(&first.receipt),
            )?;
            return Ok(UploadExecution::Failed(UploadFailure {
                source_event,
                reason: reason.to_string(),
                receipt_ids: first.receipt_ids,
            }));
        }
    };
    if first_proof.stale_chips.is_empty() {
        let completed = append_upload_completed(
            journal,
            &first_started,
            &input.request_id,
            &first_proof,
            &first.receipt,
        )?;
        return Ok(UploadExecution::Ready(UploadStage {
            completed_event: completed,
            proof: first_proof,
            upload_attempt_id: first_id,
            receipt_ids: first.receipt_ids,
        }));
    }

    let mismatch =
        append_upload_mismatch(journal, &first_started, &input.request_id, &first_proof)?;
    let clear_id = child_operation_id(operation_id, "clear")?;
    let clear_command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &clear_id,
        })?;
    let clear = match invoke::<ClearData>(
        &input.config.state_root,
        &clear_command,
        &clear_request(
            identity(cohort, &clear_id, &input.request_id, &input.run_id, slot_id),
            page,
            &first_id,
            &clear_id,
            &first_proof.stale_chips,
        ),
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            let source_event = append_upload_failed(
                journal,
                &mismatch,
                &input.request_id,
                &first_id,
                0,
                "contract.invalid_provider_envelope",
                None,
            )?;
            return Ok(UploadExecution::Failed(UploadFailure {
                source_event,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: first.receipt_ids,
            }));
        }
    };
    let clear_reason = if clear.data.observed_page_binding.as_ref() != Some(page) {
        Some("binding.mismatch")
    } else if !clear.ok {
        clear.provider_reason.as_deref()
    } else if clear.data.failure_reason.as_deref() != clear.provider_reason.as_deref()
        || clear.provider_reason.is_some()
        || !clear.data.attempted_chip_keys.is_empty()
        || clear.data.clear_attempt_id != clear_id
    {
        Some("contract.invalid_provider_envelope")
    } else {
        None
    };
    if let Some(reason) = clear_reason {
        let source_event = append_upload_failed(
            journal,
            &mismatch,
            &input.request_id,
            &first_id,
            0,
            reason,
            Some(&clear.receipt),
        )?;
        let mut receipt_ids = first.receipt_ids;
        receipt_ids.extend(clear.receipt_ids);
        return Ok(UploadExecution::Failed(UploadFailure {
            source_event,
            reason: reason.to_string(),
            receipt_ids,
        }));
    }
    let cleared = append_upload_cleared(
        journal,
        &mismatch,
        &input.request_id,
        &first_id,
        &clear_id,
        &clear.data.cleared_chips,
        &clear.receipt,
    )?;
    let retry_id = child_operation_id(operation_id, "upload1")?;
    let retry_started = append_upload_started(
        journal,
        &cleared,
        &input.request_id,
        &retry_id,
        1,
        &assets.attachment_set.set_sha256,
        &page.root_binding_hash,
    )?;
    let retry = match invoke_upload(
        input,
        &request_key,
        slot_id,
        cohort,
        page,
        assets,
        &retry_id,
        1,
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            let source_event = append_upload_failed(
                journal,
                &retry_started,
                &input.request_id,
                &retry_id,
                1,
                "contract.invalid_provider_envelope",
                None,
            )?;
            let mut receipt_ids = first.receipt_ids;
            receipt_ids.extend(clear.receipt_ids);
            return Ok(UploadExecution::Failed(UploadFailure {
                source_event,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids,
            }));
        }
    };
    let proof = match require_proof(&retry, page, &retry_id, 1) {
        Ok(proof) => proof,
        Err(reason) => {
            let source_event = append_upload_failed(
                journal,
                &retry_started,
                &input.request_id,
                &retry_id,
                1,
                reason,
                Some(&retry.receipt),
            )?;
            let mut receipt_ids = first.receipt_ids;
            receipt_ids.extend(clear.receipt_ids);
            receipt_ids.extend(retry.receipt_ids);
            return Ok(UploadExecution::Failed(UploadFailure {
                source_event,
                reason: reason.to_string(),
                receipt_ids,
            }));
        }
    };
    if !proof.stale_chips.is_empty() {
        let source_event = append_upload_failed(
            journal,
            &retry_started,
            &input.request_id,
            &retry_id,
            1,
            "upload.stale_chip_uncleared",
            Some(&retry.receipt),
        )?;
        let mut receipt_ids = first.receipt_ids;
        receipt_ids.extend(clear.receipt_ids);
        receipt_ids.extend(retry.receipt_ids);
        return Ok(UploadExecution::Failed(UploadFailure {
            source_event,
            reason: "upload.stale_chip_uncleared".to_string(),
            receipt_ids,
        }));
    }
    let completed = append_upload_completed(
        journal,
        &retry_started,
        &input.request_id,
        &proof,
        &retry.receipt,
    )?;
    let mut receipt_ids = first.receipt_ids;
    receipt_ids.extend(clear.receipt_ids);
    receipt_ids.extend(retry.receipt_ids);
    Ok(UploadExecution::Ready(UploadStage {
        completed_event: completed,
        proof,
        upload_attempt_id: retry_id,
        receipt_ids,
    }))
}

#[allow(clippy::too_many_arguments)]
fn invoke_upload(
    input: &RequestRunInput,
    request_key: &str,
    slot_id: &str,
    cohort: &str,
    page: &PageBindingEcho,
    assets: &StagedFreshAssets,
    operation_id: &str,
    retry_index: u8,
    limits: FreshProviderLimits,
) -> Result<super::r13_provider::FreshProviderResult<UploadData>, FreshRunError> {
    let command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key,
            operation_id,
        })?;
    Ok(invoke::<UploadData>(
        &input.config.state_root,
        &command,
        &upload_request(
            identity(
                cohort,
                operation_id,
                &input.request_id,
                &input.run_id,
                slot_id,
            ),
            page,
            &assets.attachment_set,
            operation_id,
            retry_index,
        ),
        limits,
    )?)
}

fn require_proof(
    result: &super::r13_provider::FreshProviderResult<UploadData>,
    page: &PageBindingEcho,
    operation_id: &str,
    retry_index: u8,
) -> Result<UploadProof, &'static str> {
    if result.data.observed_page_binding.as_ref() != Some(page) {
        return Err("binding.mismatch");
    }
    if !result.ok && result.provider_reason.as_deref() != Some("upload.stale_chip_mismatch") {
        return match result.provider_reason.as_deref() {
            Some("upload.stale_chip_uncleared") => Err("upload.stale_chip_uncleared"),
            Some("upload.incomplete") => Err("upload.incomplete"),
            Some("upload.chip_removal_failed") => Err("upload.chip_removal_failed"),
            Some("contract.invalid_provider_envelope") => Err("contract.invalid_provider_envelope"),
            Some("binding.mismatch") => Err("binding.mismatch"),
            _ => Err("contract.invalid_provider_envelope"),
        };
    }
    let proof = result
        .data
        .upload_proof
        .clone()
        .ok_or("contract.invalid_provider_envelope")?;
    if proof.upload_attempt_id != operation_id
        || proof.retry_index != retry_index
        || result.data.failure_reason.as_deref() != result.provider_reason.as_deref()
        || (result.ok && result.provider_reason.is_some())
        || (!result.ok && result.provider_reason.is_none())
    {
        return Err("contract.invalid_provider_envelope");
    }
    Ok(proof)
}
