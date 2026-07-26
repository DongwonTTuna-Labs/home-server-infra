use crate::contracts::browser::{Effort, Model, PageBindingEcho};
use crate::contracts::events::EventEnvelope;
use crate::provider_runner::R13ProviderCommandContext;
use crate::session_ops::journal::SessionJournal;

use super::input::RequestRunInput;
use super::r13_assets::{materialize_for_slot, StagedFreshAssets};
use super::r13_events::{
    append_materialized, append_model_failed, append_model_started, append_model_verified,
    append_root_failed, append_root_observed, append_root_started, build_page_binding,
};
use super::r13_provider::{
    capture_request, identity, invoke, model_request, CaptureData, FreshProviderLimits, ModelData,
};
use super::r13_types::{child_operation_id, FreshRunError};
use super::r13_upload::{run_upload, UploadExecution, UploadStage};

pub struct BrowserStage {
    pub page: PageBindingEcho,
    pub root_event: EventEnvelope,
    pub upload: UploadStage,
    pub receipt_ids: Vec<String>,
}

pub struct BrowserFailure {
    pub source_event: EventEnvelope,
    pub reason: String,
    pub receipt_ids: Vec<String>,
    pub result_kind: &'static str,
}

pub enum BrowserPreparation {
    Ready(Box<BrowserStage>),
    Failed(Box<BrowserFailure>),
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_browser(
    input: &RequestRunInput,
    operation_id: &str,
    journal: &mut SessionJournal,
    staged: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    health: &EventEnvelope,
    slot_id: &str,
    cohort: &str,
    assets: &StagedFreshAssets,
) -> Result<BrowserPreparation, FreshRunError> {
    let request_key = format!("r-{}", input.request_id);
    let limits = FreshProviderLimits {
        timeout: input.send_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    };
    let model = parse_model(&input.model)?;
    let effort = parse_effort(&input.effort)?;

    let capture_id = child_operation_id(operation_id, "capture")?;
    let capture_started = append_root_started(
        journal,
        staged,
        health,
        &input.request_id,
        &capture_id,
        slot_id,
    )?;
    let capture_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &capture_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: append_root_failed(
                    journal,
                    &capture_started,
                    &input.request_id,
                    &capture_id,
                    "contract.invalid_provider_envelope",
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: Vec::new(),
                result_kind: "run.model_failed",
            })));
        }
    };
    let captured = match invoke::<CaptureData>(
        &input.config.state_root,
        &capture_command,
        &capture_request(
            identity(
                cohort,
                &capture_id,
                &input.request_id,
                &input.run_id,
                slot_id,
            ),
            model.clone(),
            effort.clone(),
        ),
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: append_root_failed(
                    journal,
                    &capture_started,
                    &input.request_id,
                    &capture_id,
                    "contract.invalid_provider_envelope",
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: Vec::new(),
                result_kind: "run.model_failed",
            })));
        }
    };
    if !captured.ok {
        let reported_reason = captured
            .provider_reason
            .as_deref()
            .unwrap_or("contract.invalid_provider_envelope");
        let reason = if matches!(reported_reason, "capture.ambiguous" | "capture.timeout") {
            reported_reason
        } else {
            "contract.invalid_provider_envelope"
        };
        return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
            source_event: append_root_failed(
                journal,
                &capture_started,
                &input.request_id,
                &capture_id,
                reason,
                Some(&captured.receipt),
            )?,
            reason: reason.to_string(),
            receipt_ids: captured.receipt_ids,
            result_kind: "run.model_failed",
        })));
    }
    let Some(root) = captured.data.root_binding_candidate else {
        return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
            source_event: append_root_failed(
                journal,
                &capture_started,
                &input.request_id,
                &capture_id,
                "contract.invalid_provider_envelope",
                Some(&captured.receipt),
            )?,
            reason: "contract.invalid_provider_envelope".to_string(),
            receipt_ids: captured.receipt_ids,
            result_kind: "run.model_failed",
        })));
    };
    if captured.data.failure_proof.is_some() {
        return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
            source_event: append_root_failed(
                journal,
                &capture_started,
                &input.request_id,
                &capture_id,
                "contract.invalid_provider_envelope",
                Some(&captured.receipt),
            )?,
            reason: "contract.invalid_provider_envelope".to_string(),
            receipt_ids: captured.receipt_ids,
            result_kind: "run.model_failed",
        })));
    }
    let page = match build_page_binding(&root, slot_id, cohort, lease, owner) {
        Ok(page) => page,
        Err(_) => {
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: append_root_failed(
                    journal,
                    &capture_started,
                    &input.request_id,
                    &capture_id,
                    "binding.mismatch",
                    Some(&captured.receipt),
                )?,
                reason: "binding.mismatch".to_string(),
                receipt_ids: captured.receipt_ids,
                result_kind: "run.model_failed",
            })));
        }
    };
    let root_event = append_root_observed(
        journal,
        &capture_started,
        &input.request_id,
        &capture_id,
        &root,
        &page,
    )?;

    let model_id = child_operation_id(operation_id, "model")?;
    let model_started = append_model_started(
        journal,
        &root_event,
        &input.request_id,
        &model_id,
        &model,
        &effort,
    )?;
    let model_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &model_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: append_model_failed(
                    journal,
                    &model_started,
                    &input.request_id,
                    &model_id,
                    "contract.invalid_provider_envelope",
                    None,
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: captured.receipt_ids,
                result_kind: "run.model_failed",
            })));
        }
    };
    let selected = match invoke::<ModelData>(
        &input.config.state_root,
        &model_command,
        &model_request(
            identity(cohort, &model_id, &input.request_id, &input.run_id, slot_id),
            &page,
            model,
            effort,
        ),
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: append_model_failed(
                    journal,
                    &model_started,
                    &input.request_id,
                    &model_id,
                    "contract.invalid_provider_envelope",
                    None,
                    None,
                )?,
                reason: "contract.invalid_provider_envelope".to_string(),
                receipt_ids: captured.receipt_ids,
                result_kind: "run.model_failed",
            })));
        }
    };
    if !selected.ok {
        let reason = selected
            .provider_reason
            .clone()
            .ok_or(FreshRunError::Contract("ensure-model failure reason"))?;
        let failed = append_model_failed(
            journal,
            &model_started,
            &input.request_id,
            &model_id,
            &reason,
            selected.data.failure_proof.as_ref(),
            Some(&selected.receipt),
        )?;
        let mut receipt_ids = captured.receipt_ids;
        receipt_ids.extend(selected.receipt_ids);
        return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
            source_event: failed,
            reason,
            receipt_ids,
            result_kind: "run.model_failed",
        })));
    }
    if selected.data.failure_proof.is_some()
        || selected.data.observed_page_binding.as_ref() != Some(&page)
    {
        let reason = if selected.data.observed_page_binding.as_ref() != Some(&page) {
            "binding.mismatch"
        } else {
            "contract.invalid_provider_envelope"
        };
        let failed = append_model_failed(
            journal,
            &model_started,
            &input.request_id,
            &model_id,
            reason,
            None,
            Some(&selected.receipt),
        )?;
        let mut receipt_ids = captured.receipt_ids;
        receipt_ids.extend(selected.receipt_ids);
        return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
            source_event: failed,
            reason: reason.to_string(),
            receipt_ids,
            result_kind: "run.model_failed",
        })));
    }
    let model_proof = selected
        .data
        .model_proof
        .ok_or(FreshRunError::Contract("modelProof"))?;
    let effort_proof = selected
        .data
        .effort_proof
        .ok_or(FreshRunError::Contract("effortProof"))?;
    let verified = append_model_verified(
        journal,
        &model_started,
        &input.request_id,
        &model_id,
        &model_proof,
        &effort_proof,
    )?;
    let files = materialize_for_slot(input, slot_id, assets)?;
    let materialized = append_materialized(
        journal,
        &verified,
        &input.request_id,
        slot_id,
        &input.run_id,
        &assets.attachment_set.set_sha256,
        &files,
    )?;
    let upload = run_upload(
        input,
        operation_id,
        journal,
        &materialized,
        slot_id,
        cohort,
        &page,
        assets,
        limits,
    )?;
    let upload = match upload {
        UploadExecution::Ready(upload) => upload,
        UploadExecution::Failed(failure) => {
            let mut receipt_ids = captured.receipt_ids;
            receipt_ids.extend(selected.receipt_ids);
            receipt_ids.extend(failure.receipt_ids);
            return Ok(BrowserPreparation::Failed(Box::new(BrowserFailure {
                source_event: failure.source_event,
                reason: failure.reason,
                receipt_ids,
                result_kind: "run.upload_failed",
            })));
        }
    };
    let mut receipt_ids = captured.receipt_ids;
    receipt_ids.extend(selected.receipt_ids);
    receipt_ids.extend(upload.receipt_ids.iter().cloned());
    Ok(BrowserPreparation::Ready(Box::new(BrowserStage {
        page,
        root_event,
        upload,
        receipt_ids,
    })))
}

fn parse_model(value: &str) -> Result<Model, FreshRunError> {
    match value {
        "pro" => Ok(Model::Pro),
        "xhigh" => Ok(Model::Xhigh),
        _ => Err(FreshRunError::Contract("model")),
    }
}

fn parse_effort(value: &str) -> Result<Effort, FreshRunError> {
    match value {
        "standard" => Ok(Effort::Standard),
        "high" => Ok(Effort::High),
        _ => Err(FreshRunError::Contract("effort")),
    }
}
