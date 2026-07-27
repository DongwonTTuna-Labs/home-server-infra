use std::time::Duration;

use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::contracts::cli::{CommandOutcome, CommandOutcomeError};
use crate::contracts::health::HealthStatus;
use crate::contracts::provider::ProviderIdentity;
use crate::provider_client::R13ProviderInvocationError;
use crate::provider_runner::{ProviderExecution, R13ProviderCommandContext};
use crate::records;
use crate::runtime::{DockerStatus, RuntimeProbe};
use crate::session_ops::provider::{
    build_status_request, invoke_status, ProviderLimits, RebindProviderError,
    StatusInvocationResult,
};
use crate::slots::{select_fresh_slot, AllocationCandidate, SlotConfig};

#[derive(Clone, Debug)]
pub struct PreflightInput {
    pub config: SupervisorConfig,
    pub provider_execution: ProviderExecution,
    pub slot_id: Option<String>,
    pub run_id: String,
    pub provider_timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error("preflight local state failed: {0}")]
    State(#[from] std::io::Error),
    #[error("preflight outcome failed: {0}")]
    Outcome(#[from] CommandOutcomeError),
    #[error("preflight identifier derivation failed")]
    Identifier,
}

enum StatusAttempt {
    Observed(StatusInvocationResult),
    SchemaDrift(String),
    Unreachable(String),
}

pub fn run_preflight(
    input: PreflightInput,
    runtime: &dyn RuntimeProbe,
    operation_id: &str,
) -> Result<CommandOutcome, PreflightError> {
    let slot = match select_slot(&input, runtime)? {
        Some(slot) => slot,
        None => {
            let mut outcome = CommandOutcome::select(
                "preflight",
                operation_id,
                "preflight.no_slot",
                "no slot in the configured preflight inventory is runtime-available",
                Some("preflight.no_slot".to_string()),
            )?;
            fill_known_identifiers(&mut outcome, &input, None, operation_id, Vec::new());
            return Ok(outcome);
        }
    };

    let first = invoke_status_attempt(&input, &slot, operation_id, 0);
    let retry_delay_ms = match &first {
        StatusAttempt::Observed(status) => {
            crate::allocator::health::map_health(status.health_status, status.retry_after_ms)
                .retry_after_ms
        }
        StatusAttempt::Unreachable(_) => Some(250),
        StatusAttempt::SchemaDrift(_) => None,
    };
    let mut receipt_ids = attempt_receipt_ids(&first);
    let final_attempt = if let Some(delay_ms) = retry_delay_ms {
        std::thread::sleep(Duration::from_millis(delay_ms));
        let retry_operation_id = child_operation_id(operation_id, "status-retry1")?;
        let retry = invoke_status_attempt(&input, &slot, &retry_operation_id, 1);
        receipt_ids.extend(attempt_receipt_ids(&retry));
        retry
    } else {
        first
    };

    let (result_kind, message, reason) = match final_attempt {
        StatusAttempt::SchemaDrift(message) => (
            "preflight.schema_drift",
            message,
            Some("contract.invalid_provider_envelope".to_string()),
        ),
        StatusAttempt::Unreachable(message) => (
            "preflight.unreachable",
            message,
            Some("preflight.unreachable".to_string()),
        ),
        StatusAttempt::Observed(status) if !status.ok => (
            "preflight.unreachable",
            format!(
                "the provider status probe did not produce a usable health signal: providerReason={}",
                status.failure_reason.as_deref().unwrap_or("probe.unreachable")
            ),
            Some("preflight.unreachable".to_string()),
        ),
        StatusAttempt::Observed(status) => observed_result(&status),
    };
    let mut outcome =
        CommandOutcome::select("preflight", operation_id, result_kind, message, reason)?;
    fill_known_identifiers(&mut outcome, &input, Some(&slot), operation_id, receipt_ids);
    Ok(outcome)
}

fn invoke_status_attempt(
    input: &PreflightInput,
    slot: &SlotConfig,
    operation_id: &str,
    probe_attempt: u8,
) -> StatusAttempt {
    let request_key = format!("d-{operation_id}");
    let command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &slot.slot_id.0,
            request_key: &request_key,
            operation_id,
        }) {
        Ok(command) => command,
        Err(error) => {
            return StatusAttempt::Unreachable(format!(
                "the R13 preflight provider command could not be prepared: {error}"
            ));
        }
    };
    let request = build_status_request(
        ProviderIdentity {
            cohort: crate::allocator::cohort_of(&slot.slot_id.0).map(str::to_string),
            operation_id: operation_id.to_string(),
            request_id: None,
            // The provider-request identity must satisfy the shared runId⟹requestId
            // invariant (provider r13.mjs; events.rs). Preflight/status probes carry no
            // requestId, so the provider identity carries no runId either; the CLI runId
            // still appears in the R13 envelope (set separately by the command layer).
            run_id: None,
            session_id: None,
            slot_id: slot.slot_id.0.clone(),
        },
        &slot.slot_id.0,
        probe_attempt,
    );
    match invoke_status(
        &command,
        &request,
        &input.config.state_root,
        ProviderLimits {
            timeout: input.provider_timeout,
            max_stdout_bytes: input.max_stdout_bytes,
            max_stderr_bytes: input.max_stderr_bytes,
        },
    ) {
        Ok(status) => StatusAttempt::Observed(status),
        Err(error) if invalid_provider_envelope(&error) => StatusAttempt::SchemaDrift(format!(
            "the R13 preflight provider response violated its closed contract: {error}"
        )),
        Err(error) => StatusAttempt::Unreachable(format!(
            "the R13 preflight provider did not return a usable envelope: {error}"
        )),
    }
}

fn invalid_provider_envelope(error: &RebindProviderError) -> bool {
    match error {
        RebindProviderError::Invocation(error) => matches!(
            error,
            R13ProviderInvocationError::RequestContract(_)
                | R13ProviderInvocationError::ResponseContract(_)
                | R13ProviderInvocationError::Canonical(_)
                | R13ProviderInvocationError::ExitEnvelopeMismatch { .. }
                | R13ProviderInvocationError::Receipt(_)
        ),
        RebindProviderError::Response(_)
        | RebindProviderError::Proof(_)
        | RebindProviderError::Json(_) => true,
    }
}

fn observed_result(status: &StatusInvocationResult) -> (&'static str, String, Option<String>) {
    let message = format!(
        "preflight status observed healthStatus={} dockerStatus={} modelLabel={} composerReady={}",
        status.health_status, status.docker_status, status.model_label, status.composer_ready
    );
    match status.health_status {
        HealthStatus::Ready => ("preflight.ready", message, None),
        HealthStatus::ReadyModelCorrectionRequired => {
            ("preflight.model_correction_required", message, None)
        }
        HealthStatus::LoginRequired => (
            "preflight.login_required",
            message,
            Some("provider.login_required".to_string()),
        ),
        HealthStatus::SubscriptionRequired => (
            "preflight.subscription_required",
            message,
            Some("provider.subscription_required".to_string()),
        ),
        HealthStatus::ProviderLimit => (
            "preflight.provider_limit",
            message,
            Some("provider.limit".to_string()),
        ),
        HealthStatus::SchemaDrift => (
            "preflight.schema_drift",
            message,
            Some("preflight.schema_drift".to_string()),
        ),
        HealthStatus::Unreachable | HealthStatus::Unknown => (
            "preflight.unreachable",
            message,
            Some("preflight.unreachable".to_string()),
        ),
    }
}

fn attempt_receipt_ids(attempt: &StatusAttempt) -> Vec<String> {
    match attempt {
        StatusAttempt::Observed(status) => status.receipt_ids.clone(),
        StatusAttempt::SchemaDrift(_) | StatusAttempt::Unreachable(_) => Vec::new(),
    }
}

fn fill_known_identifiers(
    outcome: &mut CommandOutcome,
    input: &PreflightInput,
    slot: Option<&SlotConfig>,
    operation_id: &str,
    receipt_ids: Vec<String>,
) {
    outcome.envelope.run_id = Some(input.run_id.clone());
    outcome.envelope.slot_id = slot
        .map(|slot| slot.slot_id.0.clone())
        .or_else(|| input.slot_id.clone());
    outcome.envelope.cohort = outcome
        .envelope
        .slot_id
        .as_deref()
        .and_then(crate::allocator::cohort_of)
        .map(str::to_string);
    outcome.envelope.evidence_root = slot
        .is_some()
        .then(|| format!("evidence/diagnostics/{operation_id}"));
    outcome.envelope.receipt_ids = receipt_ids;
}

fn select_slot(
    input: &PreflightInput,
    runtime: &dyn RuntimeProbe,
) -> Result<Option<SlotConfig>, PreflightError> {
    let inventory = crate::slots::inventory(&input.config);
    if let Some(slot_id) = &input.slot_id {
        let Some(slot) = inventory
            .into_iter()
            .find(|slot| &slot.slot_id.0 == slot_id)
        else {
            return Ok(None);
        };
        return Ok(
            matches!(runtime.observe(&slot).docker_status, DockerStatus::Running).then_some(slot),
        );
    }
    let candidates = inventory
        .iter()
        .map(|slot| AllocationCandidate {
            slot_id: slot.slot_id.clone(),
            account_group: slot.account_group.clone(),
            allocatable: matches!(runtime.observe(slot).docker_status, DockerStatus::Running),
        })
        .collect::<Vec<_>>();
    let cursor = records::read_group_cursor(&input.config.state_root)?;
    let Some(decision) = select_fresh_slot(
        &candidates,
        cursor
            .as_ref()
            .map(|record| record.last_preferred_group.as_str()),
    ) else {
        return Ok(None);
    };
    Ok(inventory
        .into_iter()
        .find(|slot| slot.slot_id == decision.slot_id))
}

fn child_operation_id(parent: &str, suffix: &str) -> Result<String, PreflightError> {
    let value = format!("{parent}.{suffix}");
    crate::contracts::ids::validate_operation_id(&value).map_err(|_| PreflightError::Identifier)?;
    Ok(value)
}
