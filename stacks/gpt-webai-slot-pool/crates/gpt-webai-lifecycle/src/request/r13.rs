use crate::allocator::scan::scan;
use crate::claims::{fencing_hash, RENEW_CADENCE_MS, RESOURCE_TTL_MS};
use crate::config::now_ms;
use crate::contracts::browser::{EvidenceRef, SessionRebindExpectation};
use crate::contracts::cli::{
    ArtifactClaimSummary, CommandOutcome, LifecycleEnvelope, RetryDirective,
};
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::h256;
use crate::contracts::projection::AllocatorRecord;
use crate::contracts::provider::ProviderIdentity;
use crate::journal::canonical::canonical_bytes;
use crate::journal::MutationGuard;
use crate::provider_runner::R13ProviderCommandContext;
use crate::session_ops::journal::{NewEvent, SessionJournal};
use crate::session_ops::provider::{
    build_poll_request, build_rebind_request, build_status_request, invoke_poll, invoke_rebind,
    invoke_status, ProviderLimits,
};
use crate::session_ops::release::{
    release_request_claim_only, release_request_resources, RequestClaimOnlyReleaseInput,
    RequestReleaseInput,
};
use crate::session_ops::runtime_r13::acquire_runtime;
use crate::session_ops::terminal::{persist_poll_terminal, PollTerminalInput, TerminalResult};
use crate::sessions::{
    new_session_record, update_session_record, write_session_record, NewSessionRecord,
    SessionRecord,
};

use super::input::RequestRunInput;
use super::r13_assets::stage_fresh_assets;
use super::r13_browser::{prepare_browser, BrowserPreparation};
use super::r13_events::{
    append_accepted, append_allocation, append_claim, append_health_observed, append_health_probe,
    append_host_staged, append_lease, append_owner, append_running,
};
use super::r13_send_flow::{send_and_bind, SendExecution};
use super::r13_types::{child_operation_id, FreshRunError};

pub fn execute_fresh_run(
    input: RequestRunInput,
    operation_id: String,
    initial_guard: MutationGuard,
) -> Result<CommandOutcome, FreshRunError> {
    let assets = stage_fresh_assets(&input)?;
    let mut journal = SessionJournal::open(
        &input.config,
        operation_id.clone(),
        Some(input.request_id.clone()),
        Some(input.run_id.clone()),
    )?;
    let accepted = append_accepted(
        &mut journal,
        &input.request_id,
        &input.model,
        &assets.prompt_sha256,
        assets.prompt_size_bytes,
        assets.attachment_set.count,
        input.artifact_expectation.as_str(),
        &initial_guard,
    )?;
    drop(initial_guard);
    let claim = append_claim(
        &mut journal,
        &accepted,
        &input.request_id,
        &operation_id,
        &input.fencing_token,
    )?;
    let staged = append_host_staged(
        &mut journal,
        &accepted,
        &claim,
        &input.request_id,
        &assets.attachment_set,
    )?;
    let before_scan = journal.replay()?;
    let mut allocator = before_scan
        .state
        .allocator
        .get("allocator")
        .cloned()
        .unwrap_or_else(|| AllocatorRecord::zeroed(staged.event_id.clone()));
    let allocation_now = now_ms();
    let scanned = scan(&mut allocator, |slot_id| {
        crate::allocator::classify_slot(&before_scan.state, slot_id, allocation_now)
    })?;
    let allocation = append_allocation(
        &mut journal,
        &staged,
        &input.request_id,
        &scanned.observations,
        before_scan
            .state
            .allocator
            .get("allocator")
            .map(|record| record.last_event_id.clone()),
    )?;
    let Some(slot_id) = scanned.granted_slot_id else {
        if allocation.candidates.len() != 10 {
            return Err(FreshRunError::Contract(
                "allocation exhaustion requires ten observations",
            ));
        }
        let exhausted_at = now_ms();
        let exhausted = journal.append(NewEvent {
            aggregate_kind: AggregateKind::Allocator,
            aggregate_id: "allocator".to_string(),
            event_type: EventType::AllocationExhausted,
            payload: serde_json::json!({
                "requestId":input.request_id,"scanOrdinalCount":10,
                "observedAtMs":exhausted_at
            }),
            predecessor_event_id: allocation
                .candidates
                .last()
                .map(|event| event.event_id.clone()),
            source_event_ids: allocation
                .candidates
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
            created_at_ms: exhausted_at,
        })?;
        let request_key = format!("r-{}", input.request_id);
        release_request_claim_only(
            &mut journal,
            RequestClaimOnlyReleaseInput {
                config: &input.config,
                operation_id: &operation_id,
                request_key: &request_key,
                request_id: &input.request_id,
                request_claim_event: &claim,
                source_event: &exhausted,
                receipt_ids: &[],
            },
        )?;
        let mut outcome = CommandOutcome::select(
            "run",
            operation_id,
            "run.queued_pool_busy",
            "all ten allocation candidates were busy; the request claim was released",
            None,
        )?;
        outcome.envelope.claim_id = Some(claim.aggregate.id.clone());
        outcome.envelope.evidence_root = Some(format!("evidence/requests/r-{}", input.request_id));
        outcome.envelope.event_ids = journal.event_ids().to_vec();
        outcome.envelope.request_id = Some(input.request_id);
        outcome.envelope.run_id = Some(input.run_id);
        outcome.envelope.retry = RetryDirective {
            budget: 0,
            delay_ms: 30_000,
            owner: Some("caller".to_string()),
            retryable: true,
        };
        return Ok(outcome);
    };
    let cohort = crate::allocator::cohort_of(&slot_id)
        .ok_or(FreshRunError::Contract("slot cohort"))?
        .to_string();
    let lease = append_lease(
        &mut journal,
        &claim,
        &allocation,
        &allocator,
        &input.request_id,
        &operation_id,
        &input.fencing_token,
    )?;

    let request_key = format!("r-{}", input.request_id);
    let status_id = child_operation_id(&operation_id, "status")?;
    let status_command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id: &slot_id,
            request_key: &request_key,
            operation_id: &status_id,
        })?;
    let acquired = acquire_runtime(
        &input.config,
        &slot_id,
        &operation_id,
        &status_command,
        &input.runtime_start_mode,
        now_ms(),
    )?;
    let owner = append_owner(
        &mut journal,
        &lease,
        &acquired,
        &slot_id,
        &input.fencing_token,
    )?;
    let slot_predecessor = before_scan
        .state
        .slots
        .get(&slot_id)
        .map(|record| record.last_event_id.clone());
    let probe = append_health_probe(
        &mut journal,
        &slot_id,
        &status_id,
        0,
        slot_predecessor,
        &lease,
        &owner,
    )?;
    let status_request = build_status_request(
        super::r13_provider::identity(
            &cohort,
            &status_id,
            &input.request_id,
            &input.run_id,
            &slot_id,
        ),
        &slot_id,
        0,
    );
    let mut status = invoke_status(
        &status_command,
        &status_request,
        &input.config.state_root,
        ProviderLimits {
            timeout: input.send_process_timeout,
            max_stdout_bytes: input.max_stdout_bytes,
            max_stderr_bytes: input.max_stderr_bytes,
        },
    )?;
    let mut observed = append_health_observed(&mut journal, &slot_id, &status_id, &probe, &status)?;
    if let Some(delay_ms) =
        crate::allocator::health::map_health(status.health_status, status.retry_after_ms)
            .retry_after_ms
    {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        let retry_id = child_operation_id(&operation_id, "status-retry1")?;
        let retry_command = input
            .provider_execution
            .r13_command(R13ProviderCommandContext {
                config: &input.config,
                slot_id: &slot_id,
                request_key: &request_key,
                operation_id: &retry_id,
            })?;
        let retry_probe = append_health_probe(
            &mut journal,
            &slot_id,
            &retry_id,
            1,
            Some(observed.event_id.clone()),
            &lease,
            &owner,
        )?;
        let retry_request = build_status_request(
            super::r13_provider::identity(
                &cohort,
                &retry_id,
                &input.request_id,
                &input.run_id,
                &slot_id,
            ),
            &slot_id,
            1,
        );
        let mut retry_status = invoke_status(
            &retry_command,
            &retry_request,
            &input.config.state_root,
            ProviderLimits {
                timeout: input.send_process_timeout,
                max_stdout_bytes: input.max_stdout_bytes,
                max_stderr_bytes: input.max_stderr_bytes,
            },
        )?;
        observed = append_health_observed(
            &mut journal,
            &slot_id,
            &retry_id,
            &retry_probe,
            &retry_status,
        )?;
        let mut receipt_ids = status.receipt_ids;
        receipt_ids.extend(retry_status.receipt_ids);
        retry_status.receipt_ids = receipt_ids;
        status = retry_status;
    }
    if !status.ok || !status.health_status.is_allocatable() {
        let receipt_ids = status.receipt_ids;
        let release_incomplete = release_request_resources(
            &mut journal,
            RequestReleaseInput {
                config: &input.config,
                operation_id: &operation_id,
                request_key: &request_key,
                request_id: &input.request_id,
                slot_id: &slot_id,
                request_claim_event: &claim,
                session_claim_event: None,
                lease_event: &lease,
                owner_event: &owner,
                source_event: &observed,
                slot_predecessor: &observed,
                acquired_runtime: &acquired,
                runtime_release_mode: &input.runtime_release_mode,
                receipt_ids: &receipt_ids,
            },
        )
        .is_err();
        let mut outcome = CommandOutcome::select(
            "run",
            operation_id,
            "run.slot_readiness_failed",
            if release_incomplete {
                "the selected slot failed the R13 readiness probe; release was attempted but did not complete"
            } else {
                "the selected slot failed the R13 readiness probe and was released"
            },
            Some("slot.readiness_failed".to_string()),
        )?;
        fill_run_failure_envelope(
            &mut outcome.envelope,
            &input,
            &slot_id,
            &cohort,
            &claim,
            &lease,
            &owner,
            journal.event_ids(),
            receipt_ids,
        );
        return Ok(outcome);
    }
    let browser = match prepare_browser(
        &input,
        &operation_id,
        &mut journal,
        &staged,
        &lease,
        &owner,
        &observed,
        &slot_id,
        &cohort,
        &assets,
    )? {
        BrowserPreparation::Ready(browser) => browser,
        BrowserPreparation::Failed(failure) => {
            let mut receipt_ids = status.receipt_ids.clone();
            receipt_ids.extend(failure.receipt_ids);
            let release_incomplete = release_request_resources(
                &mut journal,
                RequestReleaseInput {
                    config: &input.config,
                    operation_id: &operation_id,
                    request_key: &request_key,
                    request_id: &input.request_id,
                    slot_id: &slot_id,
                    request_claim_event: &claim,
                    session_claim_event: None,
                    lease_event: &lease,
                    owner_event: &owner,
                    source_event: &failure.source_event,
                    slot_predecessor: &observed,
                    acquired_runtime: &acquired,
                    runtime_release_mode: &input.runtime_release_mode,
                    receipt_ids: &receipt_ids,
                },
            )
            .is_err();
            let mut outcome = CommandOutcome::select(
                "run",
                operation_id,
                failure.result_kind,
                if release_incomplete {
                    "the fresh browser stage failed; release was attempted but did not complete"
                } else {
                    "the fresh browser stage failed and acquired resources were released"
                },
                Some(failure.reason),
            )?;
            fill_run_failure_envelope(
                &mut outcome.envelope,
                &input,
                &slot_id,
                &cohort,
                &claim,
                &lease,
                &owner,
                journal.event_ids(),
                receipt_ids,
            );
            return Ok(outcome);
        }
    };
    let sent = send_and_bind(
        &input,
        &operation_id,
        &mut journal,
        &browser.root_event,
        &slot_id,
        &cohort,
        &browser.page,
        &browser.upload,
        &assets,
    )?;
    let sent = match sent {
        SendExecution::Ready(sent) => sent,
        SendExecution::Failed(failure) => {
            let mut receipt_ids = status.receipt_ids;
            receipt_ids.extend(browser.receipt_ids);
            receipt_ids.extend(failure.receipt_ids);
            let release_incomplete = release_request_resources(
                &mut journal,
                RequestReleaseInput {
                    config: &input.config,
                    operation_id: &operation_id,
                    request_key: &request_key,
                    request_id: &input.request_id,
                    slot_id: &slot_id,
                    request_claim_event: &claim,
                    session_claim_event: None,
                    lease_event: &lease,
                    owner_event: &owner,
                    source_event: &failure.source_event,
                    slot_predecessor: &observed,
                    acquired_runtime: &acquired,
                    runtime_release_mode: &input.runtime_release_mode,
                    receipt_ids: &receipt_ids,
                },
            )
            .is_err();
            let mut outcome = CommandOutcome::select(
                "run",
                operation_id,
                failure.result_kind,
                if release_incomplete {
                    "the send stage failed; release was attempted but did not complete"
                } else {
                    "the send stage failed and acquired resources were released"
                },
                Some(failure.reason),
            )?;
            fill_run_failure_envelope(
                &mut outcome.envelope,
                &input,
                &slot_id,
                &cohort,
                &claim,
                &lease,
                &owner,
                journal.event_ids(),
                receipt_ids,
            );
            return Ok(outcome);
        }
    };
    let record = new_session_record(NewSessionRecord {
        request_id: Some(input.request_id.clone()),
        run_id: Some(input.run_id.clone()),
        session_id: sent.binding.turn_start.session_id.clone(),
        conversation_url: sent.binding.turn_start.conversation_url.clone(),
        slot_id: slot_id.clone(),
        cohort: cohort.clone(),
        page_binding_generation: 1,
    })?;
    write_session_record(&input.config.state_root, &record)?;
    let running = append_running(
        &mut journal,
        &sent.binding.turn,
        &sent.binding.binding,
        &input.request_id,
        &record.session_id,
        &sent.binding.session_binding_id,
    )?;

    let polled = poll_fresh_session(
        &input,
        &operation_id,
        &mut journal,
        &record,
        &running,
        &sent.binding.binding,
        &lease,
        &owner,
        &slot_id,
        &cohort,
    )?;
    let mut rebound_record = record.clone();
    rebound_record.page_binding_generation = polled.page_binding_generation;
    rebound_record.updated_at_ms = now_ms();
    update_session_record(&input.config.state_root, &rebound_record)?;

    let mut receipt_ids = status.receipt_ids;
    receipt_ids.extend(browser.receipt_ids);
    receipt_ids.extend(sent.receipt_ids);
    receipt_ids.extend(polled.receipt_ids);
    let primary_failed = polled.reason.is_some();
    let release = release_request_resources(
        &mut journal,
        RequestReleaseInput {
            config: &input.config,
            operation_id: &operation_id,
            request_key: &request_key,
            request_id: &input.request_id,
            slot_id: &slot_id,
            request_claim_event: &claim,
            session_claim_event: Some(&polled.session_claim),
            lease_event: &lease,
            owner_event: &owner,
            source_event: &polled.source_event,
            slot_predecessor: &observed,
            acquired_runtime: &acquired,
            runtime_release_mode: &input.runtime_release_mode,
            receipt_ids: &receipt_ids,
        },
    );
    let (result_kind, reason, message) = match release {
        Ok(release) if release.stop_failed && !primary_failed => (
            "run.release_failed",
            Some("runtime.stop_failed".to_string()),
            "fresh request completed, but stopping its runtime failed".to_string(),
        ),
        Ok(_) => (
            polled.result_kind,
            polled.reason,
            "fresh request established a real session through the R13 provider journal".to_string(),
        ),
        Err(error) if !primary_failed => (
            "run.release_failed",
            Some("run.release_failed".to_string()),
            format!("fresh request completed, but its release pipeline failed: {error}"),
        ),
        Err(error) => (
            polled.result_kind,
            polled.reason,
            format!("fresh request failed and its release pipeline also failed: {error}"),
        ),
    };
    let mut outcome = CommandOutcome::select("run", operation_id, result_kind, message, reason)?;
    fill_run_envelope(
        &mut outcome.envelope,
        &input,
        &slot_id,
        &cohort,
        &claim,
        &lease,
        &owner,
        &rebound_record,
        journal.event_ids(),
        receipt_ids,
    );
    outcome.envelope.answer_path = polled.answer_path;
    outcome.envelope.answer_sha256 = polled.answer_sha256;
    outcome.envelope.answer_size_bytes = polled.answer_size_bytes;
    outcome.envelope.answer_text = polled.answer_text;
    outcome.envelope.artifact_claims = polled.artifact_claims;
    Ok(outcome)
}

struct FreshPollStage {
    session_claim: EventEnvelope,
    source_event: EventEnvelope,
    page_binding_generation: u16,
    receipt_ids: Vec<String>,
    result_kind: &'static str,
    reason: Option<String>,
    answer_path: Option<String>,
    answer_sha256: Option<String>,
    answer_size_bytes: Option<u64>,
    answer_text: Option<String>,
    artifact_claims: Vec<ArtifactClaimSummary>,
}

#[allow(clippy::too_many_arguments)]
fn poll_fresh_session(
    input: &RequestRunInput,
    operation_id: &str,
    journal: &mut SessionJournal,
    record: &SessionRecord,
    running: &EventEnvelope,
    session_predecessor: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    slot_id: &str,
    cohort: &str,
) -> Result<FreshPollStage, FreshRunError> {
    let claim_operation_id = child_operation_id(operation_id, "poll-claim")?;
    let claim_id = crate::claims::session_operation::derive_session_operation_claim_id(
        &record.session_id,
        &claim_operation_id,
        "poll",
        1,
    )?;
    let claimed_at = now_ms();
    let session_claim = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Claim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::SessionOperationClaimGranted,
        payload: serde_json::json!({
            "claimId":claim_id,"sessionId":record.session_id,"operationKind":"poll",
            "expectedSlotId":slot_id,"expectedCohort":cohort,
            "expectedRuntimeOwnerGeneration":owner.payload["ownerGeneration"],
            "requestId":input.request_id,"runId":input.run_id,"ttlMs":RESOURCE_TTL_MS,
            "grantedAtMs":claimed_at,"renewAtMs":claimed_at+RENEW_CADENCE_MS,
            "expiresAtMs":claimed_at+RESOURCE_TTL_MS,
            "fencingTokenSha256":fencing_hash(&input.fencing_token)
        }),
        predecessor_event_id: None,
        source_event_ids: vec![running.event_id.clone()],
        created_at_ms: claimed_at,
    })?;
    let expectation = SessionRebindExpectation {
        session_id: record.session_id.clone(),
        conversation_url: record.conversation_url.clone(),
        slot_id: slot_id.to_string(),
        cohort: cohort.to_string(),
        session_operation_claim_id: Some(session_claim.aggregate.id.clone()),
        lease_id: lease.aggregate.id.clone(),
        lease_generation: 1,
        runtime_owner_id: owner.aggregate.id.clone(),
        runtime_owner_generation: owner.payload["ownerGeneration"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(FreshRunError::Contract("runtime owner generation"))?,
        runtime_incarnation_id: owner.payload["runtimeIncarnationId"]
            .as_str()
            .ok_or(FreshRunError::Contract("runtime incarnation"))?
            .to_string(),
        request_id: Some(input.request_id.clone()),
        run_id: Some(input.run_id.clone()),
        last_known_page_binding_generation: record.page_binding_generation,
    };
    let rebind_operation_id = child_operation_id(operation_id, "rebind")?;
    let rebind_started_at = now_ms();
    let rebind_started = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionRebindStarted,
        payload: serde_json::json!({
            "sessionId":record.session_id,"sessionOperationClaimId":session_claim.aggregate.id,
            "operationKind":"poll","expectationSha256":h256(canonical_bytes(&expectation)?),
            "navigationAttemptLimit":2,"hydrationDeadlineMs":90000,
            "startedAtMs":rebind_started_at
        }),
        predecessor_event_id: Some(session_predecessor.event_id.clone()),
        source_event_ids: vec![
            session_claim.event_id.clone(),
            lease.event_id.clone(),
            owner.event_id.clone(),
        ],
        created_at_ms: rebind_started_at,
    })?;
    let request_key = format!("r-{}", input.request_id);
    let rebind_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &rebind_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return fresh_session_failure(
                journal,
                record,
                session_claim,
                &rebind_started,
                "rebind",
                "contract.invalid_provider_envelope",
                None,
                Vec::new(),
            );
        }
    };
    let rebind_request = build_rebind_request(&expectation, "poll", &rebind_operation_id);
    let limits = ProviderLimits {
        timeout: input.poll_process_timeout,
        max_stdout_bytes: input.max_stdout_bytes,
        max_stderr_bytes: input.max_stderr_bytes,
    };
    let rebound_result = match invoke_rebind(
        &rebind_command,
        &rebind_request,
        &input.config.state_root,
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return fresh_session_failure(
                journal,
                record,
                session_claim,
                &rebind_started,
                "rebind",
                "contract.invalid_provider_envelope",
                None,
                Vec::new(),
            );
        }
    };
    let Some(proof) = rebound_result.proof else {
        let reason = canonical_session_failure_reason(rebound_result.failure_reason.as_deref());
        return fresh_session_failure(
            journal,
            record,
            session_claim,
            &rebind_started,
            "rebind",
            reason,
            Some(&rebound_result.receipt),
            rebound_result.receipt_ids,
        );
    };
    let hydration_outcome = match proof.validate(&expectation) {
        Ok(outcome) => outcome,
        Err(_) => {
            return fresh_session_failure(
                journal,
                record,
                session_claim,
                &rebind_started,
                "rebind",
                "binding.mismatch",
                Some(&rebound_result.receipt),
                rebound_result.receipt_ids,
            );
        }
    };
    let rebound_at = now_ms();
    let rebound = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionRebound,
        payload: serde_json::json!({
            "sessionId":record.session_id,"expectation":proof.expectation,
            "observedEcho":proof.observed_echo,
            "pageBindingGeneration":proof.page_binding_generation,
            "providerReceipt":rebound_result.receipt,"reboundAtMs":rebound_at
        }),
        predecessor_event_id: Some(rebind_started.event_id.clone()),
        source_event_ids: vec![rebind_started.event_id.clone()],
        created_at_ms: rebound_at,
    })?;
    let hydrated = append_fresh_hydration(journal, record, &rebound, &proof, hydration_outcome)?;
    let poll_operation_id = child_operation_id(operation_id, "poll")?;
    let poll_started_at = now_ms();
    let poll_started = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: input.request_id.clone(),
        event_type: EventType::PollStarted,
        payload: serde_json::json!({
            "requestId":input.request_id,"pollAttemptId":poll_operation_id,
            "sessionId":record.session_id,"pollTimeoutSeconds":input.poll_timeout_seconds,
            "startedAtMs":poll_started_at
        }),
        predecessor_event_id: Some(running.event_id.clone()),
        source_event_ids: vec![hydrated.event_id.clone()],
        created_at_ms: poll_started_at,
    })?;
    let poll_command = match input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: &input.config,
            slot_id,
            request_key: &request_key,
            operation_id: &poll_operation_id,
        }) {
        Ok(command) => command,
        Err(_) => {
            return fresh_poll_failure(
                journal,
                record,
                session_claim,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                None,
                rebound_result.receipt_ids,
            );
        }
    };
    let poll_request = build_poll_request(
        ProviderIdentity {
            cohort: Some(cohort.to_string()),
            operation_id: poll_operation_id.clone(),
            request_id: Some(input.request_id.clone()),
            run_id: Some(input.run_id.clone()),
            session_id: Some(record.session_id.clone()),
            slot_id: slot_id.to_string(),
        },
        &proof.observed_echo,
        &poll_operation_id,
        input.poll_timeout_seconds,
        input.artifact_expectation.as_str(),
    );
    let polled = match invoke_poll(
        &poll_command,
        &poll_request,
        &input.config.state_root,
        limits,
    ) {
        Ok(result) => result,
        Err(_) => {
            return fresh_poll_failure(
                journal,
                record,
                session_claim,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                None,
                rebound_result.receipt_ids,
            );
        }
    };
    let mut receipt_ids = rebound_result.receipt_ids;
    receipt_ids.extend(polled.receipt_ids.clone());
    if !polled.ok {
        let reason = canonical_poll_failure_reason(polled.provider_reason.as_deref());
        return fresh_poll_failure(
            journal,
            record,
            session_claim,
            &poll_started,
            &poll_operation_id,
            reason,
            Some(&polled.receipt),
            receipt_ids,
        );
    }
    let (
        source_event,
        result_kind,
        reason,
        answer_path,
        answer_sha256,
        answer_size_bytes,
        answer_text,
        artifact_claims,
    ) = match polled.data.poll_state.as_str() {
        "running" => {
            let observed = polled.data.observed_echo.as_ref();
            let Some(observed) = observed else {
                return fresh_poll_failure(
                    journal,
                    record,
                    session_claim,
                    &poll_started,
                    &poll_operation_id,
                    "contract.invalid_provider_envelope",
                    Some(&polled.receipt),
                    receipt_ids,
                );
            };
            if !observed.active_turn {
                return fresh_poll_failure(
                    journal,
                    record,
                    session_claim,
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
                aggregate_id: input.request_id.clone(),
                event_type: EventType::PollProgress,
                payload: serde_json::json!({
                    "requestId":input.request_id,"pollAttemptId":poll_operation_id,
                    "providerStatus":"running","activeGeneration":true,"sequenceIndex":0,
                    "pollReceipt":polled.receipt,"observedAtMs":progress_at
                }),
                predecessor_event_id: Some(poll_started.event_id.clone()),
                source_event_ids: vec![poll_started.event_id.clone()],
                created_at_ms: progress_at,
            })?;
            (
                progress,
                "run.running",
                None,
                None,
                None,
                None,
                None,
                Vec::new(),
            )
        }
        "terminal" => {
            let terminal = persist_poll_terminal(PollTerminalInput {
                config: &input.config,
                journal,
                provider_execution: &input.provider_execution,
                provider_limits: limits,
                operation_id,
                request_key: &request_key,
                request_id: &input.request_id,
                run_id: &input.run_id,
                record,
                expected: &proof.observed_echo,
                hydrated: &hydrated,
                poll_started: &poll_started,
                poll_attempt_id: &poll_operation_id,
                poll_receipt: &polled.receipt,
                poll_data: &polled.data,
                artifacts_host_dir: &poll_command.paths.artifacts_host_dir,
                artifact_expectation: input.artifact_expectation.as_str(),
            })?;
            receipt_ids.extend(terminal.receipt_ids);
            let result_kind = match terminal.result {
                TerminalResult::Success => "run.terminal_success",
                TerminalResult::OptionalZero => "run.terminal_optional_zero",
                TerminalResult::ArtifactFailed => "run.artifact_required_failed",
            };
            (
                terminal.source_event,
                result_kind,
                terminal.reason,
                terminal.answer_path,
                terminal.answer_sha256,
                terminal.answer_size_bytes,
                terminal.answer_text,
                terminal.artifact_claims,
            )
        }
        _ => {
            return fresh_poll_failure(
                journal,
                record,
                session_claim,
                &poll_started,
                &poll_operation_id,
                "contract.invalid_provider_envelope",
                Some(&polled.receipt),
                receipt_ids,
            );
        }
    };
    Ok(FreshPollStage {
        session_claim,
        source_event,
        page_binding_generation: proof.page_binding_generation,
        receipt_ids,
        result_kind,
        reason,
        answer_path,
        answer_sha256,
        answer_size_bytes,
        answer_text,
        artifact_claims,
    })
}

#[allow(clippy::too_many_arguments)]
fn fresh_session_failure(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    session_claim: EventEnvelope,
    source: &EventEnvelope,
    stage: &str,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
    receipt_ids: Vec<String>,
) -> Result<FreshPollStage, FreshRunError> {
    let failed_at = now_ms().max(source.created_at_ms);
    let failed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionOperationFailed,
        payload: serde_json::json!({
            "sessionId":record.session_id,
            "sessionOperationClaimId":session_claim.aggregate.id,
            "operationKind":"poll","stage":stage,"reason":reason,
            "providerReceipt":provider_receipt,"failedAtMs":failed_at
        }),
        predecessor_event_id: Some(source.event_id.clone()),
        source_event_ids: vec![source.event_id.clone()],
        created_at_ms: failed_at,
    })?;
    Ok(FreshPollStage {
        session_claim,
        source_event: failed,
        page_binding_generation: record.page_binding_generation,
        receipt_ids,
        result_kind: "run.poll_failed",
        reason: Some(reason.to_string()),
        answer_path: None,
        answer_sha256: None,
        answer_size_bytes: None,
        answer_text: None,
        artifact_claims: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn fresh_poll_failure(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    session_claim: EventEnvelope,
    poll_started: &EventEnvelope,
    poll_operation_id: &str,
    reason: &str,
    provider_receipt: Option<&EvidenceRef>,
    receipt_ids: Vec<String>,
) -> Result<FreshPollStage, FreshRunError> {
    let failed_at = now_ms().max(poll_started.created_at_ms);
    let failed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::Request,
        aggregate_id: record
            .request_id
            .clone()
            .ok_or(FreshRunError::Contract("poll request binding"))?,
        event_type: EventType::PollFailed,
        payload: serde_json::json!({
            "requestId":record.request_id,
            "pollAttemptId":poll_operation_id,"reason":reason,
            "providerReceipt":provider_receipt,"failedAtMs":failed_at
        }),
        predecessor_event_id: Some(poll_started.event_id.clone()),
        source_event_ids: vec![poll_started.event_id.clone()],
        created_at_ms: failed_at,
    })?;
    Ok(FreshPollStage {
        session_claim,
        source_event: failed,
        page_binding_generation: record.page_binding_generation,
        receipt_ids,
        result_kind: "run.poll_failed",
        reason: Some(reason.to_string()),
        answer_path: None,
        answer_sha256: None,
        answer_size_bytes: None,
        answer_text: None,
        artifact_claims: Vec::new(),
    })
}

fn canonical_session_failure_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("session.rebind_failed") => "session.rebind_failed",
        Some("session.pinned_slot_unavailable") => "session.pinned_slot_unavailable",
        Some("session.content_unavailable") => "session.content_unavailable",
        Some("session.url_rejected_root") => "session.url_rejected_root",
        Some("session.url_rejected_mismatch") => "session.url_rejected_mismatch",
        Some("session.hydration_timeout") => "session.hydration_timeout",
        Some("session.provider_limit" | "provider.limit") => "session.provider_limit",
        Some("session.login_required" | "provider.login_required") => "session.login_required",
        Some("session.subscription_required" | "provider.subscription_required") => {
            "session.subscription_required"
        }
        Some("session.schema_drift" | "provider.schema_drift") => "session.schema_drift",
        Some("binding.mismatch") => "binding.mismatch",
        Some("contract.invalid_provider_envelope") | None => "contract.invalid_provider_envelope",
        Some(_) => "contract.invalid_provider_envelope",
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

fn append_fresh_hydration(
    journal: &mut SessionJournal,
    record: &SessionRecord,
    rebound: &EventEnvelope,
    proof: &crate::session_rebind::RebindProof,
    outcome: crate::session_rebind::hydration::HydrationOutcome,
) -> Result<EventEnvelope, FreshRunError> {
    let mut predecessor = rebound.clone();
    for observation in &proof.hydration.observations {
        let event_at = now_ms()
            .max(observation.observed_at_ms)
            .max(predecessor.created_at_ms);
        predecessor = journal.append(NewEvent {
            aggregate_kind: AggregateKind::Session,
            aggregate_id: record.session_id.clone(),
            event_type: EventType::SessionHydrationObserved,
            payload: serde_json::json!({
                "sessionId":record.session_id,"hydrationObservation":observation,
                "sequenceIndex":observation.sequence_index,
                "remainingDeadlineMs":observation.remaining_deadline_ms,
                "observedAtMs":observation.observed_at_ms
            }),
            predecessor_event_id: Some(predecessor.event_id.clone()),
            source_event_ids: vec![predecessor.event_id.clone()],
            created_at_ms: event_at,
        })?;
    }
    let final_observation = proof
        .hydration
        .observations
        .last()
        .ok_or(FreshRunError::Contract("hydration observation missing"))?;
    let at = now_ms()
        .max(final_observation.observed_at_ms)
        .max(predecessor.created_at_ms);
    Ok(journal.append(NewEvent {
        aggregate_kind: AggregateKind::Session,
        aggregate_id: record.session_id.clone(),
        event_type: EventType::SessionHydrated,
        payload: serde_json::json!({
            "sessionId":record.session_id,"observations":proof.hydration.observations.len(),
            "terminalVisible":outcome==crate::session_rebind::hydration::HydrationOutcome::Terminal,
            "activeGeneration":outcome==crate::session_rebind::hydration::HydrationOutcome::Running,
            "contentUnavailable":false,"finalObservation":final_observation,"hydratedAtMs":at
        }),
        predecessor_event_id: Some(predecessor.event_id.clone()),
        source_event_ids: vec![predecessor.event_id],
        created_at_ms: at,
    })?)
}

#[allow(clippy::too_many_arguments)]
fn fill_run_envelope(
    envelope: &mut LifecycleEnvelope,
    input: &RequestRunInput,
    slot_id: &str,
    cohort: &str,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    record: &crate::sessions::SessionRecord,
    event_ids: &[String],
    receipt_ids: Vec<String>,
) {
    envelope.claim_id = Some(claim.aggregate.id.clone());
    envelope.cohort = Some(cohort.to_string());
    envelope.conversation_url = Some(record.conversation_url.clone());
    envelope.evidence_root = Some(format!("evidence/requests/r-{}", input.request_id));
    envelope.event_ids = event_ids.to_vec();
    envelope.lease_id = Some(lease.aggregate.id.clone());
    envelope.receipt_ids = receipt_ids;
    envelope.request_id = Some(input.request_id.clone());
    envelope.run_id = Some(input.run_id.clone());
    envelope.runtime_owner_id = Some(owner.aggregate.id.clone());
    envelope.session_id = Some(record.session_id.clone());
    envelope.slot_id = Some(slot_id.to_string());
}

#[allow(clippy::too_many_arguments)]
fn fill_run_failure_envelope(
    envelope: &mut LifecycleEnvelope,
    input: &RequestRunInput,
    slot_id: &str,
    cohort: &str,
    claim: &EventEnvelope,
    lease: &EventEnvelope,
    owner: &EventEnvelope,
    event_ids: &[String],
    receipt_ids: Vec<String>,
) {
    envelope.claim_id = Some(claim.aggregate.id.clone());
    envelope.cohort = Some(cohort.to_string());
    envelope.evidence_root = Some(format!("evidence/requests/r-{}", input.request_id));
    envelope.event_ids = event_ids.to_vec();
    envelope.lease_id = Some(lease.aggregate.id.clone());
    envelope.receipt_ids = receipt_ids;
    envelope.request_id = Some(input.request_id.clone());
    envelope.run_id = Some(input.run_id.clone());
    envelope.runtime_owner_id = Some(owner.aggregate.id.clone());
    envelope.slot_id = Some(slot_id.to_string());
}
