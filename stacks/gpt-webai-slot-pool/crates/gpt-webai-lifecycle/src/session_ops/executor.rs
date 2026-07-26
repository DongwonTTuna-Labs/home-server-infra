use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

use serde_json::json;
use thiserror::Error;

use crate::claims::{derived_id, fencing_hash, CasError, RENEW_CADENCE_MS, RESOURCE_TTL_MS};
use crate::config::{now_ms, SupervisorConfig};
use crate::contracts::browser::{EvidenceRef, SessionRebindExpectation};
use crate::contracts::cli::{CommandOutcome, CommandOutcomeError};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::h256;
use crate::contracts::provider::ProviderIdentity;
use crate::journal::canonical::canonical_bytes;
use crate::journal::{EventStore, EventStoreError};
use crate::provider_runner::{ProviderExecution, ProviderRunnerError, R13ProviderCommandContext};
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::session_rebind::hydration::HydrationOutcome;
use crate::sessions::{
    read_session_record, update_session_record, SessionRecord, SessionRecordError,
};

use super::artifacts::{recover_artifacts, ArtifactPipelineError, ArtifactPipelineInput};
use super::journal::{NewEvent, SessionJournal, SessionJournalError};
use super::provider::{
    build_poll_request, build_rebind_request, build_status_request, invoke_poll, invoke_rebind,
    invoke_status, ProviderLimits, RebindProviderError, StatusInvocationResult,
};
use super::release::{
    release_session_partial, release_session_resources, SessionPartialReleaseInput,
    SessionReleaseError, SessionReleaseInput,
};
use super::runtime_r13::{acquire_runtime, SessionRuntimeR13Error};
use super::terminal::{
    persist_poll_terminal, PollTerminalInput, TerminalPipelineError, TerminalResult,
};

pub struct SessionExecutorInput {
    pub config: SupervisorConfig,
    pub operation_id: String,
    pub session_id: String,
    pub fencing_token: String,
    pub provider_execution: ProviderExecution,
    pub runtime_start_mode: RuntimeStartMode,
    pub runtime_release_mode: RuntimeReleaseMode,
    pub provider_limits: ProviderLimits,
}

#[derive(Debug, Error)]
pub enum SessionExecutorError {
    #[error("session journal failed: {0}")]
    Journal(#[from] SessionJournalError),
    #[error("session provider command failed: {0}")]
    ProviderCommand(#[from] ProviderRunnerError),
    #[error("session provider invocation failed: {0}")]
    Provider(#[from] RebindProviderError),
    #[error("session runtime failed: {0}")]
    Runtime(#[from] SessionRuntimeR13Error),
    #[error("session release failed: {0}")]
    Release(#[from] SessionReleaseError),
    #[error("session identifier derivation failed: {0}")]
    Cas(#[from] CasError),
    #[error("session output contract failed: {0}")]
    Outcome(#[from] CommandOutcomeError),
    #[error("session record failed: {0}")]
    Session(#[from] SessionRecordError),
    #[error("session rebind proof failed: {0}")]
    Rebind(#[from] crate::session_rebind::SessionRebindError),
    #[error("session answer io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("session answer JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session answer contract failed: {0}")]
    Answer(&'static str),
    #[error("session event store failed: {0}")]
    Store(#[from] EventStoreError),
    #[error("session terminal pipeline failed: {0}")]
    Terminal(#[from] TerminalPipelineError),
    #[error("session artifact pipeline failed: {0}")]
    Artifact(#[from] ArtifactPipelineError),
}

pub fn execute_show(input: SessionExecutorInput) -> Result<CommandOutcome, SessionExecutorError> {
    let record = match read_session_record(&input.config.state_root, &input.session_id) {
        Ok(record) => record,
        Err(SessionRecordError::Missing(_)) => {
            return Ok(pre_acquisition_failure(
                &input,
                "show",
                "show.unknown_session",
                "session.missing",
                "persisted session is missing",
            )?);
        }
        Err(error) => return Err(error.into()),
    };
    let request_key = record
        .request_id
        .as_ref()
        .map(|request_id| format!("r-{request_id}"))
        .unwrap_or_else(|| format!("s-{}", record.session_id));
    let mut journal = SessionJournal::open(
        &input.config,
        input.operation_id.clone(),
        record.request_id.clone(),
        record.run_id.clone(),
    )?;
    let initial = journal.replay()?;
    if initial
        .state
        .claims
        .values()
        .any(|claim| claim.status == "active" && claim.subject_id == record.session_id)
    {
        return Ok(pre_acquisition_failure(
            &input,
            "show",
            "show.claim_conflict",
            "session.claim_conflict",
            "another persisted-session operation owns the session claim",
        )?);
    }
    let prior_terminal_hash = initial
        .state
        .sessions
        .get(&record.session_id)
        .and_then(|session| session.terminal_answer_sha256.clone());
    let session_predecessor =
        journal.aggregate_tail_event_id(AggregateKind::Session, &record.session_id)?;
    let slot_predecessor = initial
        .state
        .slots
        .get(&record.slot_id)
        .map(|slot| slot.last_event_id.clone());

    let claim = match append_claim(&mut journal, &record, &input, "show", None) {
        Ok(claim) => claim,
        Err(error) if is_first_mutation_lock_contention(&error) => {
            return Ok(pre_acquisition_failure(
                &input,
                "show",
                "show.lock_contended",
                "lock.contended",
                "the lifecycle state-store mutation lock is contended",
            )?);
        }
        Err(error) => return Err(error),
    };
    let lease = match append_lease(&mut journal, &record, &input, &claim) {
        Ok(lease) => lease,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                None,
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "show",
                    stage: "lease",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &claim,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let status_operation_id = child_operation_id(&input.operation_id, "status")?;
    let status_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &status_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "show",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let acquired = match acquire_runtime(
        &input.config,
        &record.slot_id,
        &input.operation_id,
        &status_command,
        &input.runtime_start_mode,
        now_ms(),
    ) {
        Ok(acquired) => acquired,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "show",
                    stage: "runtime",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let owner = append_owner(&mut journal, &record, &input, &claim, &lease, &acquired)?;
    let health_execution = invoke_health_with_retry(
        &mut journal,
        &record,
        &input,
        &request_key,
        &lease,
        &owner,
        slot_predecessor,
        &status_operation_id,
        &status_command,
    )?;
    let (status, health) = match health_execution {
        HealthExecution::Observed(status, health) => (status, health),
        HealthExecution::Failed {
            health,
            receipt_ids,
        } => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
    };
    if let Some(reason) = readiness_failure_reason(&status) {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "show",
                stage: "runtime",
                reason,
                source_event: &health,
                provider_receipt: Some(&status.receipt),
                receipt_ids: status.receipt_ids,
            },
        );
    }

    let expectation = SessionRebindExpectation {
        session_id: record.session_id.clone(),
        conversation_url: record.conversation_url.clone(),
        slot_id: record.slot_id.clone(),
        cohort: record.cohort.clone(),
        session_operation_claim_id: Some(claim.aggregate.id.clone()),
        lease_id: lease.aggregate.id.clone(),
        lease_generation: 1,
        runtime_owner_id: acquired.owner_id.clone(),
        runtime_owner_generation: acquired.owner_generation,
        runtime_incarnation_id: acquired.runtime_incarnation_id.clone(),
        request_id: record.request_id.clone(),
        run_id: record.run_id.clone(),
        last_known_page_binding_generation: record.page_binding_generation,
    };
    let rebind_operation_id = match child_operation_id(&input.operation_id, "rebind") {
        Ok(operation_id) => operation_id,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_started = match append_rebind_started(
        &mut journal,
        &record,
        &expectation,
        session_predecessor,
        &claim,
        &lease,
        &owner,
        "show",
    ) {
        Ok(started) => started,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &rebind_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_request = build_rebind_request(&expectation, "show", &rebind_operation_id);
    let invoked = match invoke_rebind(
        &rebind_command,
        &rebind_request,
        &input.config.state_root,
        input.provider_limits,
    ) {
        Ok(invoked) => invoked,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let mut receipt_ids = status.receipt_ids;
    receipt_ids.extend(invoked.receipt_ids.clone());
    if let Some(reported) = invoked.failure_reason.as_deref() {
        let reason = canonical_session_failure_reason(reported);
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "show",
                stage: "rebind",
                reason,
                source_event: &rebind_started,
                provider_receipt: Some(&invoked.receipt),
                receipt_ids,
            },
        );
    }
    let proof = match invoked.proof.as_ref() {
        Some(proof) => proof,
        None => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let outcome = match proof.validate(&expectation) {
        Ok(outcome) => outcome,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "rebind",
                    reason: "binding.mismatch",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let rebound = append_rebound(
        &mut journal,
        &record,
        &rebind_started,
        proof,
        &invoked.receipt,
    )?;
    let hydrated = append_hydration(&mut journal, &record, &rebound, proof, outcome)?;

    let mut updated = record.clone();
    updated.page_binding_generation = proof.page_binding_generation;
    updated.updated_at_ms = now_ms();
    if update_session_record(&input.config.state_root, &updated).is_err() {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "show",
                stage: "content",
                reason: "session.content_unavailable",
                source_event: &hydrated,
                provider_receipt: None,
                receipt_ids,
            },
        );
    }

    let classification = match classify_show(
        &input,
        &record,
        prior_terminal_hash.as_deref(),
        proof,
        &rebind_command.paths.artifacts_host_dir,
    ) {
        Ok(classification) => classification,
        Err(SessionExecutorError::Answer("session.request_binding_missing")) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "content",
                    reason: "session.request_binding_missing",
                    source_event: &hydrated,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "show",
                    stage: "content",
                    reason: "session.content_unavailable",
                    source_event: &hydrated,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
    };
    let release = release_session_resources(
        &mut journal,
        SessionReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key: &request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: &claim,
            lease_event: &lease,
            owner_event: &owner,
            source_event: &hydrated,
            slot_predecessor: &health,
            acquired_runtime: &acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &receipt_ids,
        },
    )?;
    if release.stop_failed {
        return build_session_outcome(
            &input,
            &record,
            &request_key,
            &journal,
            &receipt_ids,
            &claim,
            &lease,
            &acquired.owner_id,
            "show",
            "show.release_failed",
            Some("runtime.stop_failed".to_string()),
        );
    }
    build_show_outcome(
        &input,
        &record,
        &request_key,
        &journal,
        &receipt_ids,
        &claim,
        &lease,
        &acquired.owner_id,
        classification,
    )
}

pub fn execute_resume(
    input: SessionExecutorInput,
    poll_timeout_seconds: u64,
) -> Result<CommandOutcome, SessionExecutorError> {
    let record = match read_session_record(&input.config.state_root, &input.session_id) {
        Ok(record) => record,
        Err(SessionRecordError::Missing(_)) => {
            return Ok(pre_acquisition_failure(
                &input,
                "resume",
                "resume.unknown_session",
                "session.missing",
                "persisted session is missing",
            )?);
        }
        Err(error) => return Err(error.into()),
    };
    let (request_id, run_id) = match record.request_binding() {
        Ok((request_id, run_id)) => (request_id.to_string(), run_id.to_string()),
        Err(_) => {
            return Ok(pre_acquisition_failure(
                &input,
                "resume",
                "resume.request_binding_missing",
                "session.request_binding_missing",
                "persisted session has no request/run binding",
            )?);
        }
    };
    let request_key = format!("r-{request_id}");
    let mut journal = SessionJournal::open(
        &input.config,
        input.operation_id.clone(),
        Some(request_id.clone()),
        Some(run_id.clone()),
    )?;
    let initial = journal.replay()?;
    if initial
        .state
        .claims
        .values()
        .any(|claim| claim.status == "active" && claim.subject_id == record.session_id)
    {
        return Ok(pre_acquisition_failure(
            &input,
            "resume",
            "resume.claim_conflict",
            "session.claim_conflict",
            "another persisted-session operation owns the session claim",
        )?);
    }
    let Some(request_predecessor) =
        journal.aggregate_tail_event_id(AggregateKind::Request, &request_id)?
    else {
        return Ok(pre_acquisition_failure(
            &input,
            "resume",
            "resume.request_binding_missing",
            "session.request_binding_missing",
            "the persisted request binding has no durable request projection",
        )?);
    };
    let session_predecessor =
        journal.aggregate_tail_event_id(AggregateKind::Session, &record.session_id)?;
    let slot_predecessor = initial
        .state
        .slots
        .get(&record.slot_id)
        .map(|slot| slot.last_event_id.clone());
    let artifact_expectation = request_artifact_expectation(&input.config, &request_id)?;

    let claim = match append_claim(&mut journal, &record, &input, "resume", None) {
        Ok(claim) => claim,
        Err(error) if is_first_mutation_lock_contention(&error) => {
            return Ok(pre_acquisition_failure(
                &input,
                "resume",
                "resume.lock_contended",
                "lock.contended",
                "the lifecycle state-store mutation lock is contended",
            )?);
        }
        Err(error) => return Err(error),
    };
    let lease = match append_lease(&mut journal, &record, &input, &claim) {
        Ok(lease) => lease,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                None,
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "resume",
                    stage: "lease",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &claim,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let status_operation_id = child_operation_id(&input.operation_id, "status")?;
    let status_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &status_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "resume",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let acquired = match acquire_runtime(
        &input.config,
        &record.slot_id,
        &input.operation_id,
        &status_command,
        &input.runtime_start_mode,
        now_ms(),
    ) {
        Ok(acquired) => acquired,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "resume",
                    stage: "runtime",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let owner = append_owner(&mut journal, &record, &input, &claim, &lease, &acquired)?;
    let health_execution = invoke_health_with_retry(
        &mut journal,
        &record,
        &input,
        &request_key,
        &lease,
        &owner,
        slot_predecessor,
        &status_operation_id,
        &status_command,
    )?;
    let (status, health) = match health_execution {
        HealthExecution::Observed(status, health) => (status, health),
        HealthExecution::Failed {
            health,
            receipt_ids,
        } => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
    };
    if let Some(reason) = readiness_failure_reason(&status) {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "resume",
                stage: "runtime",
                reason,
                source_event: &health,
                provider_receipt: Some(&status.receipt),
                receipt_ids: status.receipt_ids,
            },
        );
    }
    let expectation = SessionRebindExpectation {
        session_id: record.session_id.clone(),
        conversation_url: record.conversation_url.clone(),
        slot_id: record.slot_id.clone(),
        cohort: record.cohort.clone(),
        session_operation_claim_id: Some(claim.aggregate.id.clone()),
        lease_id: lease.aggregate.id.clone(),
        lease_generation: 1,
        runtime_owner_id: acquired.owner_id.clone(),
        runtime_owner_generation: acquired.owner_generation,
        runtime_incarnation_id: acquired.runtime_incarnation_id.clone(),
        request_id: Some(request_id.clone()),
        run_id: Some(run_id.clone()),
        last_known_page_binding_generation: record.page_binding_generation,
    };
    let rebind_operation_id = match child_operation_id(&input.operation_id, "rebind") {
        Ok(operation_id) => operation_id,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_started = match append_rebind_started(
        &mut journal,
        &record,
        &expectation,
        session_predecessor,
        &claim,
        &lease,
        &owner,
        "resume",
    ) {
        Ok(started) => started,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &rebind_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_request = build_rebind_request(&expectation, "resume", &rebind_operation_id);
    let invoked = match invoke_rebind(
        &rebind_command,
        &rebind_request,
        &input.config.state_root,
        input.provider_limits,
    ) {
        Ok(invoked) => invoked,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let mut receipt_ids = status.receipt_ids;
    receipt_ids.extend(invoked.receipt_ids.clone());
    if let Some(reported) = invoked.failure_reason.as_deref() {
        let reason = canonical_session_failure_reason(reported);
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "resume",
                stage: "rebind",
                reason,
                source_event: &rebind_started,
                provider_receipt: Some(&invoked.receipt),
                receipt_ids,
            },
        );
    }
    let proof = match invoked.proof.as_ref() {
        Some(proof) => proof,
        None => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let hydration_outcome = match proof.validate(&expectation) {
        Ok(outcome) => outcome,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "binding.mismatch",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let rebound = match append_rebound(
        &mut journal,
        &record,
        &rebind_started,
        proof,
        &invoked.receipt,
    ) {
        Ok(rebound) => rebound,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let hydrated = match append_hydration(&mut journal, &record, &rebound, proof, hydration_outcome)
    {
        Ok(hydrated) => hydrated,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "resume",
                    stage: "hydration",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebound,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let mut updated = record.clone();
    updated.page_binding_generation = proof.page_binding_generation;
    updated.updated_at_ms = now_ms();
    if update_session_record(&input.config.state_root, &updated).is_err() {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "resume",
                stage: "content",
                reason: "session.content_unavailable",
                source_event: &hydrated,
                provider_receipt: None,
                receipt_ids,
            },
        );
    }

    let poll_operation_id = child_operation_id(&input.operation_id, "poll")?;
    let poll_started_at = now_ms();
    let poll_started = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.clone(),
        event_type: EventType::PollStarted,
        payload: json!({
            "requestId":request_id,"pollAttemptId":poll_operation_id,
            "sessionId":record.session_id,"pollTimeoutSeconds":poll_timeout_seconds,
            "startedAtMs":poll_started_at
        }),
        predecessor_event_id: Some(request_predecessor),
        source_event_ids: vec![hydrated.event_id.clone()],
        created_at_ms: poll_started_at,
    })?;
    let poll_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &poll_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_resume_poll_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                None,
                receipt_ids,
            );
        }
    };
    let poll_request = build_poll_request(
        ProviderIdentity {
            cohort: Some(record.cohort.clone()),
            operation_id: poll_operation_id.clone(),
            request_id: Some(request_id.clone()),
            run_id: Some(run_id.clone()),
            session_id: Some(record.session_id.clone()),
            slot_id: record.slot_id.clone(),
        },
        &proof.observed_echo,
        &poll_operation_id,
        poll_timeout_seconds,
        &artifact_expectation,
    );
    let polled = match invoke_poll(
        &poll_command,
        &poll_request,
        &input.config.state_root,
        input.provider_limits,
    ) {
        Ok(polled) => polled,
        Err(_) => {
            return finish_resume_poll_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                None,
                receipt_ids,
            );
        }
    };
    receipt_ids.extend(polled.receipt_ids.clone());
    if !polled.ok {
        let reason = canonical_poll_failure_reason(polled.provider_reason.as_deref());
        return finish_resume_poll_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            &poll_started,
            &poll_operation_id,
            reason,
            Some(&polled.receipt),
            receipt_ids,
        );
    }
    let mut answer_path = None;
    let mut answer_sha256 = None;
    let mut answer_size_bytes = None;
    let mut answer_text = None;
    let mut artifact_claims = Vec::new();
    let mut reason = None;
    let (source, result_kind) = match polled.data.poll_state.as_str() {
        "running" => {
            let observed = match polled.data.observed_echo.as_ref() {
                Some(observed) => observed,
                None => {
                    return finish_resume_poll_failure(
                        &mut journal,
                        &input,
                        &record,
                        &request_key,
                        &claim,
                        &lease,
                        &owner,
                        &health,
                        &acquired,
                        &poll_started,
                        &poll_operation_id,
                        "contract.invalid_provider_envelope",
                        Some(&polled.receipt),
                        receipt_ids,
                    );
                }
            };
            if !observed.active_turn {
                return finish_resume_poll_failure(
                    &mut journal,
                    &input,
                    &record,
                    &request_key,
                    &claim,
                    &lease,
                    &owner,
                    &health,
                    &acquired,
                    &poll_started,
                    &poll_operation_id,
                    "contract.invalid_provider_envelope",
                    Some(&polled.receipt),
                    receipt_ids,
                );
            }
            let progress_at = now_ms();
            let progress = journal.append(NewEvent {
                aggregate_kind: AggregateKind::Request,
                aggregate_id: request_id.clone(),
                event_type: EventType::PollProgress,
                payload: json!({
                    "requestId":request_id,"pollAttemptId":poll_operation_id,
                    "providerStatus":"running","activeGeneration":true,"sequenceIndex":0,
                    "pollReceipt":polled.receipt,"observedAtMs":progress_at
                }),
                predecessor_event_id: Some(poll_started.event_id.clone()),
                source_event_ids: vec![poll_started.event_id.clone()],
                created_at_ms: progress_at,
            })?;
            (progress, "resume.running")
        }
        "terminal" => {
            let terminal = persist_poll_terminal(PollTerminalInput {
                config: &input.config,
                journal: &mut journal,
                provider_execution: &input.provider_execution,
                provider_limits: input.provider_limits,
                operation_id: &input.operation_id,
                request_key: &request_key,
                request_id: &request_id,
                run_id: &run_id,
                record: &record,
                expected: &proof.observed_echo,
                hydrated: &hydrated,
                poll_started: &poll_started,
                poll_attempt_id: &poll_operation_id,
                poll_receipt: &polled.receipt,
                poll_data: &polled.data,
                artifacts_host_dir: &poll_command.paths.artifacts_host_dir,
                artifact_expectation: &artifact_expectation,
            })?;
            receipt_ids.extend(terminal.receipt_ids);
            answer_path = terminal.answer_path;
            answer_sha256 = terminal.answer_sha256;
            answer_size_bytes = terminal.answer_size_bytes;
            answer_text = terminal.answer_text;
            artifact_claims = terminal.artifact_claims;
            reason = terminal.reason;
            let result_kind = match terminal.result {
                TerminalResult::Success => "resume.terminal_success",
                TerminalResult::OptionalZero => "resume.terminal_optional_zero",
                TerminalResult::ArtifactFailed => "resume.artifact_required_failed",
            };
            (terminal.source_event, result_kind)
        }
        _ => {
            return finish_resume_poll_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                Some(&polled.receipt),
                receipt_ids,
            );
        }
    };
    let release = release_session_resources(
        &mut journal,
        SessionReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key: &request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: &claim,
            lease_event: &lease,
            owner_event: &owner,
            source_event: &source,
            slot_predecessor: &health,
            acquired_runtime: &acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &receipt_ids,
        },
    )?;
    let (result_kind, reason) = if release.stop_failed && reason.is_none() {
        (
            "resume.release_failed",
            Some("runtime.stop_failed".to_string()),
        )
    } else {
        (result_kind, reason)
    };
    let mut outcome = build_session_outcome(
        &input,
        &record,
        &request_key,
        &journal,
        &receipt_ids,
        &claim,
        &lease,
        &acquired.owner_id,
        "resume",
        result_kind,
        reason,
    )?;
    outcome.envelope.answer_path = answer_path;
    outcome.envelope.answer_sha256 = answer_sha256;
    outcome.envelope.answer_size_bytes = answer_size_bytes;
    outcome.envelope.answer_text = answer_text;
    outcome.envelope.artifact_claims = artifact_claims;
    Ok(outcome)
}

pub fn execute_download(
    input: SessionExecutorInput,
    artifact_expectation: &str,
) -> Result<CommandOutcome, SessionExecutorError> {
    let record = match read_session_record(&input.config.state_root, &input.session_id) {
        Ok(record) => record,
        Err(SessionRecordError::Missing(_)) => {
            return Ok(pre_acquisition_failure(
                &input,
                "download",
                "download.unknown_session",
                "session.missing",
                "persisted session is missing",
            )?);
        }
        Err(error) => return Err(error.into()),
    };
    let request_key = record
        .request_id
        .as_ref()
        .map(|request_id| format!("r-{request_id}"))
        .unwrap_or_else(|| format!("s-{}", record.session_id));
    let mut journal = SessionJournal::open(
        &input.config,
        input.operation_id.clone(),
        record.request_id.clone(),
        record.run_id.clone(),
    )?;
    let initial = journal.replay()?;
    if initial
        .state
        .claims
        .values()
        .any(|claim| claim.status == "active" && claim.subject_id == record.session_id)
    {
        return Ok(pre_acquisition_failure(
            &input,
            "download",
            "download.claim_conflict",
            "session.claim_conflict",
            "another persisted-session operation owns the session claim",
        )?);
    }
    let session_predecessor =
        journal.aggregate_tail_event_id(AggregateKind::Session, &record.session_id)?;
    let slot_predecessor = initial
        .state
        .slots
        .get(&record.slot_id)
        .map(|slot| slot.last_event_id.clone());

    let claim = match append_claim(&mut journal, &record, &input, "download", None) {
        Ok(claim) => claim,
        Err(error) if is_first_mutation_lock_contention(&error) => {
            return Ok(pre_acquisition_failure(
                &input,
                "download",
                "download.lock_contended",
                "lock.contended",
                "the lifecycle state-store mutation lock is contended",
            )?);
        }
        Err(error) => return Err(error),
    };
    let lease = match append_lease(&mut journal, &record, &input, &claim) {
        Ok(lease) => lease,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                None,
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "download",
                    stage: "lease",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &claim,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let status_operation_id = child_operation_id(&input.operation_id, "status")?;
    let status_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &status_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "download",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let acquired = match acquire_runtime(
        &input.config,
        &record.slot_id,
        &input.operation_id,
        &status_command,
        &input.runtime_start_mode,
        now_ms(),
    ) {
        Ok(acquired) => acquired,
        Err(_) => {
            return finish_partial_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                Some(&lease),
                slot_predecessor.as_deref(),
                SessionStageFailure {
                    command: "download",
                    stage: "runtime",
                    reason: "session.pinned_slot_unavailable",
                    source_event: &lease,
                    provider_receipt: None,
                    receipt_ids: Vec::new(),
                },
            );
        }
    };
    let owner = append_owner(&mut journal, &record, &input, &claim, &lease, &acquired)?;
    let health_execution = invoke_health_with_retry(
        &mut journal,
        &record,
        &input,
        &request_key,
        &lease,
        &owner,
        slot_predecessor,
        &status_operation_id,
        &status_command,
    )?;
    let (status, health) = match health_execution {
        HealthExecution::Observed(status, health) => (status, health),
        HealthExecution::Failed {
            health,
            receipt_ids,
        } => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "runtime",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
    };
    if let Some(reason) = readiness_failure_reason(&status) {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "download",
                stage: "runtime",
                reason,
                source_event: &health,
                provider_receipt: Some(&status.receipt),
                receipt_ids: status.receipt_ids,
            },
        );
    }

    let expectation = SessionRebindExpectation {
        session_id: record.session_id.clone(),
        conversation_url: record.conversation_url.clone(),
        slot_id: record.slot_id.clone(),
        cohort: record.cohort.clone(),
        session_operation_claim_id: Some(claim.aggregate.id.clone()),
        lease_id: lease.aggregate.id.clone(),
        lease_generation: 1,
        runtime_owner_id: acquired.owner_id.clone(),
        runtime_owner_generation: acquired.owner_generation,
        runtime_incarnation_id: acquired.runtime_incarnation_id.clone(),
        request_id: record.request_id.clone(),
        run_id: record.run_id.clone(),
        last_known_page_binding_generation: record.page_binding_generation,
    };
    let rebind_operation_id = match child_operation_id(&input.operation_id, "rebind") {
        Ok(operation_id) => operation_id,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_started = match append_rebind_started(
        &mut journal,
        &record,
        &expectation,
        session_predecessor,
        &claim,
        &lease,
        &owner,
        "download",
    ) {
        Ok(started) => started,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &health,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &record.slot_id,
            request_key: &request_key,
            operation_id: &rebind_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let rebind_request = build_rebind_request(&expectation, "download", &rebind_operation_id);
    let invoked = match invoke_rebind(
        &rebind_command,
        &rebind_request,
        &input.config.state_root,
        input.provider_limits,
    ) {
        Ok(invoked) => invoked,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: None,
                    receipt_ids: status.receipt_ids,
                },
            );
        }
    };
    let mut receipt_ids = status.receipt_ids;
    receipt_ids.extend(invoked.receipt_ids.clone());
    if let Some(reported) = invoked.failure_reason.as_deref() {
        let reason = canonical_session_failure_reason(reported);
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "download",
                stage: "rebind",
                reason,
                source_event: &rebind_started,
                provider_receipt: Some(&invoked.receipt),
                receipt_ids,
            },
        );
    }
    let proof = match invoked.proof.as_ref() {
        Some(proof) => proof,
        None => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let hydration_outcome = match proof.validate(&expectation) {
        Ok(outcome) => outcome,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "binding.mismatch",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let rebound = match append_rebound(
        &mut journal,
        &record,
        &rebind_started,
        proof,
        &invoked.receipt,
    ) {
        Ok(rebound) => rebound,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "rebind",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebind_started,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let hydrated = match append_hydration(&mut journal, &record, &rebound, proof, hydration_outcome)
    {
        Ok(hydrated) => hydrated,
        Err(_) => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "hydration",
                    reason: "contract.invalid_provider_envelope",
                    source_event: &rebound,
                    provider_receipt: Some(&invoked.receipt),
                    receipt_ids,
                },
            );
        }
    };
    let terminal_answer = match proof.terminal_answer.as_ref() {
        Some(answer) => answer,
        None => {
            return finish_full_session_failure(
                &mut journal,
                &input,
                &record,
                &request_key,
                &claim,
                &lease,
                &owner,
                &health,
                &acquired,
                SessionStageFailure {
                    command: "download",
                    stage: "content",
                    reason: "session.content_unavailable",
                    source_event: &hydrated,
                    provider_receipt: None,
                    receipt_ids,
                },
            );
        }
    };
    let mut updated = record.clone();
    updated.page_binding_generation = proof.page_binding_generation;
    updated.updated_at_ms = now_ms();
    if update_session_record(&input.config.state_root, &updated).is_err() {
        return finish_full_session_failure(
            &mut journal,
            &input,
            &record,
            &request_key,
            &claim,
            &lease,
            &owner,
            &health,
            &acquired,
            SessionStageFailure {
                command: "download",
                stage: "content",
                reason: "session.content_unavailable",
                source_event: &hydrated,
                provider_receipt: None,
                receipt_ids,
            },
        );
    }

    let artifact = recover_artifacts(ArtifactPipelineInput {
        config: &input.config,
        journal: &mut journal,
        provider_execution: &input.provider_execution,
        provider_limits: input.provider_limits,
        operation_id: &input.operation_id,
        request_key: &request_key,
        request_id: record.request_id.as_deref(),
        run_id: record.run_id.as_deref(),
        record: &record,
        expected: &proof.observed_echo,
        source_event: &hydrated,
        terminal_assistant_turn_id: &terminal_answer.terminal_assistant_turn_id,
        expectation: artifact_expectation,
    })?;
    receipt_ids.extend(artifact.receipt_ids);
    let release = release_session_resources(
        &mut journal,
        SessionReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key: &request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: &claim,
            lease_event: &lease,
            owner_event: &owner,
            source_event: &artifact.terminal_event,
            slot_predecessor: &health,
            acquired_runtime: &acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &receipt_ids,
        },
    )?;
    let mut result_kind = match artifact.failure_reason.as_deref() {
        None if artifact.optional_zero => "download.optional_zero",
        None => "download.completed",
        Some("artifact.required_zero") => "download.controls_absent_required",
        Some("artifact.controls_ambiguous" | "artifact.bottom_unverified") => {
            "download.ambiguous_controls"
        }
        Some("artifact.download_timeout" | "artifact.event_unrecoverable") => {
            "download.event_timeout"
        }
        Some("artifact.integrity_failed" | "artifact.path_unsafe") => "download.integrity_failed",
        Some(_) => return Err(SessionExecutorError::Answer("download artifact result")),
    };
    let mut reason = artifact.failure_reason;
    if release.stop_failed && reason.is_none() {
        result_kind = "download.release_failed";
        reason = Some("runtime.stop_failed".to_string());
    }
    let mut outcome = build_session_outcome(
        &input,
        &record,
        &request_key,
        &journal,
        &receipt_ids,
        &claim,
        &lease,
        &acquired.owner_id,
        "download",
        result_kind,
        reason,
    )?;
    outcome.envelope.artifact_claims = vec![artifact.summary];
    Ok(outcome)
}

struct ShowClassification {
    result_kind: &'static str,
    answer_path: Option<String>,
    answer_sha256: Option<String>,
    answer_size_bytes: Option<u64>,
    answer_text: Option<String>,
}

fn classify_show(
    input: &SessionExecutorInput,
    record: &SessionRecord,
    prior_terminal_hash: Option<&str>,
    proof: &crate::session_rebind::RebindProof,
    artifacts_root: &std::path::Path,
) -> Result<ShowClassification, SessionExecutorError> {
    if proof.observed_echo.active_turn {
        return Ok(ShowClassification {
            result_kind: "show.running",
            answer_path: None,
            answer_sha256: None,
            answer_size_bytes: None,
            answer_text: None,
        });
    }
    let answer = proof
        .terminal_answer
        .as_ref()
        .ok_or(SessionExecutorError::Answer("terminal answer missing"))?;
    if prior_terminal_hash == Some(answer.answer_sha256.as_str()) {
        return Ok(ShowClassification {
            result_kind: "show.idle",
            answer_path: None,
            answer_sha256: None,
            answer_size_bytes: None,
            answer_text: None,
        });
    }
    let (Some(request_id), Some(_run_id)) = (&record.request_id, &record.run_id) else {
        return Err(SessionExecutorError::Answer(
            "session.request_binding_missing",
        ));
    };
    let source = artifacts_root.join(&answer.answer_rel_path);
    let relative = format!("answers/r-{request_id}/{}.answer.md", input.operation_id);
    let target = input.config.state_root.join(&relative);
    let bytes = copy_verified_answer(
        &input.config.state_root,
        &source,
        &target,
        &answer.answer_sha256,
        answer.answer_size_bytes,
    )?;
    let answer_text = (bytes.len() <= 65_536)
        .then(|| String::from_utf8(bytes).ok())
        .flatten();
    Ok(ShowClassification {
        result_kind: "show.terminal",
        answer_path: Some(relative),
        answer_sha256: Some(answer.answer_sha256.clone()),
        answer_size_bytes: Some(answer.answer_size_bytes),
        answer_text,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_show_outcome(
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    journal: &SessionJournal,
    receipt_ids: &[String],
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner_id: &str,
    classification: ShowClassification,
) -> Result<CommandOutcome, SessionExecutorError> {
    let mut outcome = CommandOutcome::select(
        "show",
        input.operation_id.clone(),
        classification.result_kind,
        "persisted session rebound and hydrated through the R13 provider",
        None,
    )?;
    let envelope = &mut outcome.envelope;
    envelope.answer_path = classification.answer_path;
    envelope.answer_sha256 = classification.answer_sha256;
    envelope.answer_size_bytes = classification.answer_size_bytes;
    envelope.answer_text = classification.answer_text;
    envelope.claim_id = Some(claim.aggregate.id.clone());
    envelope.cohort = Some(record.cohort.clone());
    envelope.conversation_url = Some(record.conversation_url.clone());
    envelope.evidence_root = Some(format!("evidence/requests/{request_key}"));
    envelope.event_ids = journal.event_ids().to_vec();
    envelope.lease_id = Some(lease.aggregate.id.clone());
    envelope.receipt_ids = receipt_ids.to_vec();
    envelope.request_id = record.request_id.clone();
    envelope.run_id = record.run_id.clone();
    envelope.runtime_owner_id = Some(owner_id.to_string());
    envelope.session_id = Some(record.session_id.clone());
    envelope.slot_id = Some(record.slot_id.clone());
    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
fn build_session_outcome(
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    journal: &SessionJournal,
    receipt_ids: &[String],
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner_id: &str,
    command: &str,
    result_kind: &str,
    reason: Option<String>,
) -> Result<CommandOutcome, SessionExecutorError> {
    let mut outcome = CommandOutcome::select(
        command,
        input.operation_id.clone(),
        result_kind,
        "persisted session operation completed through the R13 provider journal",
        reason,
    )?;
    let envelope = &mut outcome.envelope;
    envelope.claim_id = Some(claim.aggregate.id.clone());
    envelope.cohort = Some(record.cohort.clone());
    envelope.conversation_url = Some(record.conversation_url.clone());
    envelope.evidence_root = Some(format!("evidence/requests/{request_key}"));
    envelope.event_ids = journal.event_ids().to_vec();
    envelope.lease_id = Some(lease.aggregate.id.clone());
    envelope.receipt_ids = receipt_ids.to_vec();
    envelope.request_id = record.request_id.clone();
    envelope.run_id = record.run_id.clone();
    envelope.runtime_owner_id = Some(owner_id.to_string());
    envelope.session_id = Some(record.session_id.clone());
    envelope.slot_id = Some(record.slot_id.clone());
    Ok(outcome)
}

struct SessionStageFailure<'a> {
    command: &'static str,
    stage: &'static str,
    reason: &'a str,
    source_event: &'a EventEnvelope,
    provider_receipt: Option<&'a EvidenceRef>,
    receipt_ids: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn finish_partial_session_failure(
    journal: &mut SessionJournal,
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    claim: &EventEnvelope,
    lease: Option<&EventEnvelope>,
    slot_predecessor_event_id: Option<&str>,
    failure: SessionStageFailure<'_>,
) -> Result<CommandOutcome, SessionExecutorError> {
    let failed = append_session_operation_failed(journal, record, claim, &failure)?;
    let release_completed = release_session_partial(
        journal,
        SessionPartialReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: claim,
            lease_event: lease,
            source_event: &failed,
            slot_predecessor_event_id,
            receipt_ids: &failure.receipt_ids,
        },
    )
    .is_ok();
    build_session_failure_outcome(
        input,
        record,
        request_key,
        journal,
        claim,
        lease,
        None,
        failure.command,
        session_failure_result_kind(failure.command, failure.stage, failure.reason),
        failure.reason,
        &failure.receipt_ids,
        release_completed,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_full_session_failure(
    journal: &mut SessionJournal,
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    health: &EventEnvelope,
    acquired: &super::runtime_r13::AcquiredRuntime,
    failure: SessionStageFailure<'_>,
) -> Result<CommandOutcome, SessionExecutorError> {
    let failed = append_session_operation_failed(journal, record, claim, &failure)?;
    let release_completed = release_session_resources(
        journal,
        SessionReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: claim,
            lease_event: lease,
            owner_event: owner,
            source_event: &failed,
            slot_predecessor: health,
            acquired_runtime: acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &failure.receipt_ids,
        },
    )
    .is_ok();
    build_session_failure_outcome(
        input,
        record,
        request_key,
        journal,
        claim,
        Some(lease),
        Some(&acquired.owner_id),
        failure.command,
        session_failure_result_kind(failure.command, failure.stage, failure.reason),
        failure.reason,
        &failure.receipt_ids,
        release_completed,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_resume_poll_failure(
    journal: &mut SessionJournal,
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    health: &EventEnvelope,
    acquired: &super::runtime_r13::AcquiredRuntime,
    poll_started: &EventEnvelope,
    poll_attempt_id: &str,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
    receipt_ids: Vec<String>,
) -> Result<CommandOutcome, SessionExecutorError> {
    let request_id = record
        .request_id
        .as_deref()
        .ok_or(SessionExecutorError::Answer(
            "resume request binding missing",
        ))?;
    let failed_at = now_ms().max(poll_started.created_at_ms);
    let failed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: request_id.to_string(),
        event_type: EventType::PollFailed,
        payload: json!({
            "requestId":request_id,"pollAttemptId":poll_attempt_id,"reason":reason,
            "providerReceipt":provider_receipt,"failedAtMs":failed_at
        }),
        predecessor_event_id: Some(poll_started.event_id.clone()),
        source_event_ids: vec![poll_started.event_id.clone()],
        created_at_ms: failed_at,
    })?;
    let _ = release_session_resources(
        journal,
        SessionReleaseInput {
            config: &input.config,
            operation_id: &input.operation_id,
            request_key,
            session_id: &record.session_id,
            slot_id: &record.slot_id,
            claim_event: claim,
            lease_event: lease,
            owner_event: owner,
            source_event: &failed,
            slot_predecessor: health,
            acquired_runtime: acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &receipt_ids,
        },
    );
    build_session_outcome(
        input,
        record,
        request_key,
        journal,
        &receipt_ids,
        claim,
        lease,
        &acquired.owner_id,
        "resume",
        "resume.poll_failed",
        Some(reason.to_string()),
    )
}

fn append_session_operation_failed(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    claim: &EventEnvelope,
    failure: &SessionStageFailure<'_>,
) -> Result<EventEnvelope, SessionExecutorError> {
    let failed_at = now_ms().max(failure.source_event.created_at_ms);
    let predecessor_event_id =
        journal.aggregate_tail_event_id(AggregateKind::Session, &record.session_id)?;
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionOperationFailed,
        payload: json!({
            "sessionId":record.session_id,"sessionOperationClaimId":claim.aggregate.id,
            "operationKind":failure.command,"stage":failure.stage,"reason":failure.reason,
            "providerReceipt":failure.provider_receipt,"failedAtMs":failed_at
        }),
        predecessor_event_id,
        source_event_ids: vec![failure.source_event.event_id.clone()],
        created_at_ms: failed_at,
    })?)
}

fn session_failure_result_kind(
    command: &'static str,
    stage: &'static str,
    reason: &str,
) -> &'static str {
    match (command, reason) {
        ("show", "contract.invalid_provider_envelope" | "binding.mismatch") => {
            "show.content_unavailable"
        }
        ("resume", "contract.invalid_provider_envelope" | "binding.mismatch") => {
            "resume.content_unavailable"
        }
        ("download", "contract.invalid_provider_envelope" | "binding.mismatch") => {
            "download.content_unavailable"
        }
        ("show", "session.request_binding_missing") => "show.request_binding_missing",
        ("show", "session.pinned_slot_unavailable") => "show.pinned_slot_unavailable",
        ("resume", "session.pinned_slot_unavailable") => "resume.pinned_slot_unavailable",
        ("download", "session.pinned_slot_unavailable") => "download.pinned_slot_unavailable",
        ("show", "session.url_rejected_root" | "session.url_rejected_mismatch") => {
            "show.url_rejected"
        }
        ("resume", "session.url_rejected_root" | "session.url_rejected_mismatch") => {
            "resume.url_rejected"
        }
        ("download", "session.url_rejected_root" | "session.url_rejected_mismatch") => {
            "download.url_rejected"
        }
        (
            "show",
            "session.provider_limit"
            | "session.login_required"
            | "session.subscription_required"
            | "session.schema_drift",
        ) => "show.provider_blocked",
        (
            "resume",
            "session.provider_limit"
            | "session.login_required"
            | "session.subscription_required"
            | "session.schema_drift",
        ) => "resume.provider_blocked",
        (
            "download",
            "session.provider_limit"
            | "session.login_required"
            | "session.subscription_required"
            | "session.schema_drift",
        ) => "download.provider_blocked",
        ("show", _) if matches!(stage, "lease" | "runtime") => "show.pinned_slot_unavailable",
        ("resume", _) if matches!(stage, "lease" | "runtime") => "resume.pinned_slot_unavailable",
        ("download", _) if matches!(stage, "lease" | "runtime") => {
            "download.pinned_slot_unavailable"
        }
        ("show", _) => "show.content_unavailable",
        ("resume", _) => "resume.content_unavailable",
        ("download", _) => "download.content_unavailable",
        _ => unreachable!("closed session operation command"),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_session_failure_outcome(
    input: &SessionExecutorInput,
    record: &SessionRecord,
    request_key: &str,
    journal: &SessionJournal,
    claim: &EventEnvelope,
    lease: Option<&EventEnvelope>,
    owner_id: Option<&str>,
    command: &str,
    result_kind: &str,
    reason: &str,
    receipt_ids: &[String],
    release_completed: bool,
) -> Result<CommandOutcome, SessionExecutorError> {
    let message = if release_completed {
        "persisted session operation failed and acquired resources were released"
    } else {
        "persisted session operation failed; resource release remains incomplete"
    };
    let mut outcome = CommandOutcome::select(
        command,
        input.operation_id.clone(),
        result_kind,
        message,
        Some(reason.to_string()),
    )?;
    let envelope = &mut outcome.envelope;
    envelope.claim_id = Some(claim.aggregate.id.clone());
    envelope.cohort = Some(record.cohort.clone());
    envelope.conversation_url = Some(record.conversation_url.clone());
    envelope.evidence_root = Some(format!("evidence/requests/{request_key}"));
    envelope.event_ids = journal.event_ids().to_vec();
    envelope.lease_id = lease.map(|event| event.aggregate.id.clone());
    envelope.receipt_ids = receipt_ids.to_vec();
    envelope.request_id = record.request_id.clone();
    envelope.run_id = record.run_id.clone();
    envelope.runtime_owner_id = owner_id.map(str::to_string);
    envelope.session_id = Some(record.session_id.clone());
    envelope.slot_id = Some(record.slot_id.clone());
    Ok(outcome)
}

fn request_artifact_expectation(
    config: &SupervisorConfig,
    request_id: &str,
) -> Result<String, SessionExecutorError> {
    let mut matches = EventStore::new(&config.state_root)
        .load_all()?
        .into_iter()
        .filter(|event| {
            event.event_type == EventType::RequestAccepted
                && event.aggregate.id == request_id
                && event.request_id.as_deref() == Some(request_id)
        });
    let event = matches
        .next()
        .ok_or(SessionExecutorError::Answer("request acceptance missing"))?;
    if matches.next().is_some() {
        return Err(SessionExecutorError::Answer("duplicate request acceptance"));
    }
    let expectation = event
        .payload
        .get("artifactExpectation")
        .and_then(serde_json::Value::as_str)
        .ok_or(SessionExecutorError::Answer("artifact expectation missing"))?;
    if !matches!(expectation, "none" | "optional" | "required" | "claimed") {
        return Err(SessionExecutorError::Answer("artifact expectation invalid"));
    }
    Ok(expectation.to_string())
}

fn pre_acquisition_failure(
    input: &SessionExecutorInput,
    command: &str,
    result_kind: &str,
    reason: &str,
    message: &str,
) -> Result<CommandOutcome, CommandOutcomeError> {
    let mut outcome = CommandOutcome::select(
        command,
        input.operation_id.clone(),
        result_kind,
        message,
        Some(reason.to_string()),
    )?;
    outcome.envelope.session_id = Some(input.session_id.clone());
    Ok(outcome)
}

fn is_first_mutation_lock_contention(error: &SessionExecutorError) -> bool {
    matches!(
        error,
        SessionExecutorError::Journal(SessionJournalError::Head(
            crate::journal::HeadError::LockContended(_)
        ))
    )
}

fn append_claim(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    input: &SessionExecutorInput,
    operation_kind: &str,
    source_event_id: Option<String>,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    let claim_id = crate::claims::session_operation::derive_session_operation_claim_id(
        &record.session_id,
        &input.operation_id,
        operation_kind,
        1,
    )?;
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Claim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::SessionOperationClaimGranted,
        payload: json!({
            "claimId":claim_id,"sessionId":record.session_id,"operationKind":operation_kind,
            "expectedSlotId":record.slot_id,"expectedCohort":record.cohort,
            "expectedRuntimeOwnerGeneration":null,"requestId":record.request_id,
            "runId":record.run_id,"ttlMs":RESOURCE_TTL_MS,"grantedAtMs":at,
            "renewAtMs":at+RENEW_CADENCE_MS,"expiresAtMs":at+RESOURCE_TTL_MS,
            "fencingTokenSha256":fencing_hash(&input.fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: source_event_id.into_iter().collect(),
        created_at_ms: at,
    })?)
}

fn append_lease(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    input: &SessionExecutorInput,
    claim: &EventEnvelope,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    let lease_id = derived_id(
        "lease_",
        &json!([
            "pr72.persisted-session-lease.r13.v1",
            record.session_id,
            input.operation_id,
            1
        ]),
    )?;
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Lease,
        aggregate_id: lease_id.clone(),
        event_type: EventType::PersistedSessionLeaseGranted,
        payload: json!({
            "leaseId":lease_id,"claimId":claim.aggregate.id,"slotId":record.slot_id,
            "cohort":record.cohort,"leaseGeneration":1,"reason":"persisted_session",
            "grantedAtMs":at,"renewAtMs":at+RENEW_CADENCE_MS,
            "expiresAtMs":at+RESOURCE_TTL_MS,
            "fencingTokenSha256":fencing_hash(&input.fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: vec![claim.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn append_owner(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    input: &SessionExecutorInput,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    acquired: &super::runtime_r13::AcquiredRuntime,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::RuntimeOwner,
        aggregate_id: acquired.owner_id.clone(),
        event_type: EventType::SessionRuntimeOwnershipGranted,
        payload: json!({
            "runtimeOwnerId":acquired.owner_id,"sessionId":record.session_id,
            "slotId":record.slot_id,"leaseId":lease.aggregate.id,
            "ownerGeneration":acquired.owner_generation,
            "runtimeIncarnationId":acquired.runtime_incarnation_id,
            "dockerStatus":acquired.docker_status,"startReceipt":acquired.start_receipt,
            "grantedAtMs":at,"renewAtMs":at+RENEW_CADENCE_MS,
            "expiresAtMs":at+RESOURCE_TTL_MS,
            "fencingTokenSha256":fencing_hash(&input.fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: vec![claim.event_id.clone(), lease.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn append_probe(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    probe_id: &str,
    retry_index: u8,
    predecessor_event_id: Option<String>,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: record.slot_id.clone(),
        event_type: EventType::SlotHealthProbeStarted,
        payload: json!({
            "slotId":record.slot_id,"probeId":probe_id,"dockerStatus":"running",
            "deadlineMs":15000,"retryIndex":retry_index,"startedAtMs":at
        }),
        predecessor_event_id,
        source_event_ids: vec![lease.event_id.clone(), owner.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn append_health(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    probe: &EventEnvelope,
    status: &StatusInvocationResult,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    let (cooldown, allocatable) = health_policy(status);
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: record.slot_id.clone(),
        event_type: EventType::SlotHealthObserved,
        payload: json!({
            "slotId":record.slot_id,"probeId":probe.payload["probeId"],
            "healthStatus":status.health_status,"dockerStatus":status.docker_status,
            "cooldownMs":cooldown,"allocatable":allocatable,
            "evidenceRefs":[status.receipt],"observedAtMs":at
        }),
        predecessor_event_id: Some(probe.event_id.clone()),
        source_event_ids: vec![probe.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn append_failed_health(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    probe: &EventEnvelope,
) -> Result<EventEnvelope, SessionExecutorError> {
    let health_status = crate::contracts::health::HealthStatus::Unreachable;
    let decision = crate::allocator::health::map_health(health_status, None);
    let at = now_ms().max(probe.created_at_ms);
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Slot,
        aggregate_id: record.slot_id.clone(),
        event_type: EventType::SlotHealthObserved,
        payload: json!({
            "slotId":record.slot_id,"probeId":probe.payload["probeId"],
            "healthStatus":health_status,"dockerStatus":"unknown",
            "cooldownMs":decision.cooldown_ms,"allocatable":false,
            "evidenceRefs":[],"observedAtMs":at
        }),
        predecessor_event_id: Some(probe.event_id.clone()),
        source_event_ids: vec![probe.event_id.clone()],
        created_at_ms: at,
    })?)
}

enum HealthExecution {
    Observed(StatusInvocationResult, EventEnvelope),
    Failed {
        health: EventEnvelope,
        receipt_ids: Vec<String>,
    },
}

#[allow(clippy::too_many_arguments)]
fn invoke_health_with_retry(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    input: &SessionExecutorInput,
    request_key: &str,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    slot_predecessor: Option<String>,
    first_operation_id: &str,
    first_command: &crate::provider_runner::R13ProviderCommand,
) -> Result<HealthExecution, SessionExecutorError> {
    let probe = append_probe(
        journal,
        record,
        first_operation_id,
        0,
        slot_predecessor,
        lease,
        owner,
    )?;
    let request = build_status_request(
        ProviderIdentity {
            cohort: Some(record.cohort.clone()),
            operation_id: first_operation_id.to_string(),
            request_id: record.request_id.clone(),
            run_id: record.run_id.clone(),
            session_id: Some(record.session_id.clone()),
            slot_id: record.slot_id.clone(),
        },
        &record.slot_id,
        0,
    );
    let mut status = match invoke_status(
        first_command,
        &request,
        &input.config.state_root,
        input.provider_limits,
    ) {
        Ok(status) => status,
        Err(_) => {
            return Ok(HealthExecution::Failed {
                health: append_failed_health(journal, record, &probe)?,
                receipt_ids: Vec::new(),
            });
        }
    };
    let mut health = append_health(journal, record, &probe, &status)?;
    if let Some(delay_ms) =
        crate::allocator::health::map_health(status.health_status, status.retry_after_ms)
            .retry_after_ms
    {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        let retry_operation_id = child_operation_id(&input.operation_id, "status-retry1")?;
        let retry_probe = append_probe(
            journal,
            record,
            &retry_operation_id,
            1,
            Some(health.event_id.clone()),
            lease,
            owner,
        )?;
        let retry_command = match input
            .provider_execution
            .r13_command(R13ProviderCommandContext {
                config: &input.config,
                slot_id: &record.slot_id,
                request_key,
                operation_id: &retry_operation_id,
            }) {
            Ok(command) => command,
            Err(_) => {
                return Ok(HealthExecution::Failed {
                    health: append_failed_health(journal, record, &retry_probe)?,
                    receipt_ids: status.receipt_ids,
                });
            }
        };
        let retry_request = build_status_request(
            ProviderIdentity {
                cohort: Some(record.cohort.clone()),
                operation_id: retry_operation_id,
                request_id: record.request_id.clone(),
                run_id: record.run_id.clone(),
                session_id: Some(record.session_id.clone()),
                slot_id: record.slot_id.clone(),
            },
            &record.slot_id,
            1,
        );
        let mut retry_status = match invoke_status(
            &retry_command,
            &retry_request,
            &input.config.state_root,
            input.provider_limits,
        ) {
            Ok(status) => status,
            Err(_) => {
                return Ok(HealthExecution::Failed {
                    health: append_failed_health(journal, record, &retry_probe)?,
                    receipt_ids: status.receipt_ids,
                });
            }
        };
        health = append_health(journal, record, &retry_probe, &retry_status)?;
        let mut receipt_ids = status.receipt_ids;
        receipt_ids.extend(retry_status.receipt_ids);
        retry_status.receipt_ids = receipt_ids;
        status = retry_status;
    }
    Ok(HealthExecution::Observed(status, health))
}

#[allow(clippy::too_many_arguments)]
fn append_rebind_started(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    expectation: &SessionRebindExpectation,
    predecessor_event_id: Option<String>,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    operation_kind: &str,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    let expectation_hash = h256(canonical_bytes(expectation)?);
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionRebindStarted,
        payload: json!({
            "sessionId":record.session_id,"sessionOperationClaimId":claim.aggregate.id,
            "operationKind":operation_kind,"expectationSha256":expectation_hash,
            "navigationAttemptLimit":2,"hydrationDeadlineMs":90000,"startedAtMs":at
        }),
        predecessor_event_id,
        source_event_ids: vec![
            claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        created_at_ms: at,
    })?)
}

fn append_rebound(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    started: &EventEnvelope,
    proof: &crate::session_rebind::RebindProof,
    receipt: &crate::contracts::browser::EvidenceRef,
) -> Result<EventEnvelope, SessionExecutorError> {
    let at = now_ms();
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionRebound,
        payload: json!({
            "sessionId":record.session_id,"expectation":proof.expectation,
            "observedEcho":proof.observed_echo,"pageBindingGeneration":proof.page_binding_generation,
            "providerReceipt":receipt,"reboundAtMs":at
        }),
        predecessor_event_id: Some(started.event_id.clone()),
        source_event_ids: vec![started.event_id.clone()],
        created_at_ms: at,
    })?)
}

fn append_hydration(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    rebound: &EventEnvelope,
    proof: &crate::session_rebind::RebindProof,
    outcome: HydrationOutcome,
) -> Result<EventEnvelope, SessionExecutorError> {
    let mut predecessor = rebound.clone();
    for observation in &proof.hydration.observations {
        let event_at = now_ms()
            .max(observation.observed_at_ms)
            .max(predecessor.created_at_ms);
        let event = journal.append(NewEvent {
            aggregate_kind: AggregateKind::Session,
            aggregate_id: record.session_id.clone(),
            event_type: EventType::SessionHydrationObserved,
            payload: json!({
                "sessionId":record.session_id,"hydrationObservation":observation,
                "sequenceIndex":observation.sequence_index,
                "remainingDeadlineMs":observation.remaining_deadline_ms,
                "observedAtMs":observation.observed_at_ms
            }),
            predecessor_event_id: Some(predecessor.event_id.clone()),
            source_event_ids: vec![predecessor.event_id.clone()],
            created_at_ms: event_at,
        })?;
        predecessor = event;
    }
    let final_observation =
        proof
            .hydration
            .observations
            .last()
            .ok_or(SessionExecutorError::Answer(
                "hydration observation missing",
            ))?;
    let at = now_ms()
        .max(final_observation.observed_at_ms)
        .max(predecessor.created_at_ms);
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionHydrated,
        payload: json!({
            "sessionId":record.session_id,"observations":proof.hydration.observations.len(),
            "terminalVisible":outcome==HydrationOutcome::Terminal,
            "activeGeneration":outcome==HydrationOutcome::Running,
            "contentUnavailable":false,"finalObservation":final_observation,
            "hydratedAtMs":at
        }),
        predecessor_event_id: Some(predecessor.event_id.clone()),
        source_event_ids: vec![predecessor.event_id],
        created_at_ms: at,
    })?)
}

fn readiness_failure_reason(status: &StatusInvocationResult) -> Option<&'static str> {
    let observation_ready = match status.health_status {
        crate::contracts::health::HealthStatus::Ready => {
            status.composer_ready && status.model_label == "pro"
        }
        crate::contracts::health::HealthStatus::ReadyModelCorrectionRequired => {
            status.composer_ready && status.model_label == "non_pro"
        }
        _ => false,
    };
    if status.ok && status.failure_reason.is_none() && observation_ready {
        return None;
    }
    use crate::contracts::health::HealthStatus;
    Some(match status.health_status {
        HealthStatus::ProviderLimit => "session.provider_limit",
        HealthStatus::LoginRequired => "session.login_required",
        HealthStatus::SubscriptionRequired => "session.subscription_required",
        HealthStatus::SchemaDrift => "session.schema_drift",
        HealthStatus::Unreachable | HealthStatus::Unknown => "session.pinned_slot_unavailable",
        HealthStatus::Ready | HealthStatus::ReadyModelCorrectionRequired => {
            "contract.invalid_provider_envelope"
        }
    })
}

fn canonical_session_failure_reason(reason: &str) -> &'static str {
    match reason {
        "session.rebind_failed" => "session.rebind_failed",
        "session.pinned_slot_unavailable" => "session.pinned_slot_unavailable",
        "session.content_unavailable" => "session.content_unavailable",
        "session.url_rejected_root" => "session.url_rejected_root",
        "session.url_rejected_mismatch" => "session.url_rejected_mismatch",
        "session.hydration_timeout" => "session.hydration_timeout",
        "session.provider_limit" | "provider.limit" => "session.provider_limit",
        "session.login_required" | "provider.login_required" => "session.login_required",
        "session.subscription_required" | "provider.subscription_required" => {
            "session.subscription_required"
        }
        "session.schema_drift" | "provider.schema_drift" => "session.schema_drift",
        "binding.mismatch" => "binding.mismatch",
        "contract.invalid_provider_envelope" => "contract.invalid_provider_envelope",
        _ => "contract.invalid_provider_envelope",
    }
}

fn canonical_poll_failure_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("poll.timeout") => "poll.timeout",
        Some("provider.limit") => "provider.limit",
        Some("provider.login_required") => "provider.login_required",
        Some("provider.schema_drift") => "provider.schema_drift",
        Some("session.rebind_failed") => "session.rebind_failed",
        Some("binding.mismatch") => "binding.mismatch",
        Some("contract.invalid_provider_envelope") | None => "contract.invalid_provider_envelope",
        Some(_) => "contract.invalid_provider_envelope",
    }
}

fn health_policy(status: &StatusInvocationResult) -> (u64, bool) {
    let decision =
        crate::allocator::health::map_health(status.health_status, status.retry_after_ms);
    (decision.cooldown_ms, decision.allocatable)
}

fn child_operation_id(parent: &str, suffix: &str) -> Result<String, SessionExecutorError> {
    let value = format!("{parent}.{suffix}");
    crate::contracts::ids::validate_operation_id(&value)
        .map_err(|_| SessionExecutorError::Answer("child operation id"))?;
    Ok(value)
}

fn copy_verified_answer(
    state_root: &std::path::Path,
    source: &std::path::Path,
    target: &std::path::Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<Vec<u8>, SessionExecutorError> {
    let mut source_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(source)?;
    let metadata = source_file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(SessionExecutorError::Answer("unsafe staged answer"));
    }
    let mut bytes = Vec::new();
    source_file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected_size || h256(&bytes) != expected_sha256 {
        return Err(SessionExecutorError::Answer("staged answer digest"));
    }
    let parent = target
        .parent()
        .ok_or(SessionExecutorError::Answer("answer parent"))?;
    crate::provider_runner::create_private_directory(state_root, parent)?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            File::open(parent)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(target)? != bytes {
                return Err(SessionExecutorError::Answer("answer immutable collision"));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(bytes)
}
