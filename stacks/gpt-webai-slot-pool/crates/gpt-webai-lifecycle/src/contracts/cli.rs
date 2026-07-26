use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::ids::{validate_non_empty_text, validate_operation_id};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetryDirective {
    pub budget: u16,
    pub delay_ms: u64,
    pub owner: Option<String>,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactClaimSummary {
    pub artifact_claim_id: String,
    pub expectation: String,
    pub status: String,
    pub result: Option<String>,
    pub artifact_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LifecycleEnvelope {
    pub answer_path: Option<String>,
    pub answer_sha256: Option<String>,
    pub answer_size_bytes: Option<u64>,
    pub answer_text: Option<String>,
    pub artifact_claims: Vec<ArtifactClaimSummary>,
    pub claim_id: Option<String>,
    pub cohort: Option<String>,
    pub command: String,
    pub conversation_url: Option<String>,
    pub evidence_root: Option<String>,
    pub event_ids: Vec<String>,
    pub lease_id: Option<String>,
    pub message: String,
    pub ok: bool,
    pub operation_id: String,
    pub reason: Option<String>,
    pub receipt_ids: Vec<String>,
    pub request_id: Option<String>,
    pub result_kind: String,
    pub retry: RetryDirective,
    pub run_id: Option<String>,
    pub runtime_owner_id: Option<String>,
    pub schema: String,
    pub session_id: Option<String>,
    pub slot_id: Option<String>,
    pub status: String,
    pub terminal: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultSpec {
    pub exit_code: u8,
    pub ok: bool,
    pub reason_required: bool,
    pub terminal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutcome {
    pub envelope: LifecycleEnvelope,
    pub exit_code: u8,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandOutcomeError {
    #[error("unknown command/result pair: {command}/{result_kind}")]
    UnknownResult {
        command: String,
        result_kind: String,
    },
    #[error("invalid lifecycle operation id")]
    InvalidOperationId,
    #[error("invalid lifecycle result message")]
    InvalidMessage,
    #[error("lifecycle reason presence does not match result matrix")]
    InvalidReasonPresence,
}

const STATUS_RESULTS: &str = "status.ready status.blocked status.degraded status.state_invalid \
status.runtime_probe_failed status.lock_contended";
const PREFLIGHT_RESULTS: &str = "preflight.ready preflight.model_correction_required \
preflight.login_required preflight.subscription_required preflight.provider_limit \
preflight.unreachable preflight.schema_drift preflight.no_slot preflight.state_invalid \
preflight.lock_contended";
const RUN_RESULTS: &str = "run.running run.terminal_success run.terminal_optional_zero \
run.queued_pool_busy run.model_failed run.upload_failed run.send_failed run.send_uncertain \
run.poll_failed run.artifact_required_failed run.output_publish_failed \
run.slot_readiness_failed run.release_failed run.lock_contended";
const SHOW_RESULTS: &str = "show.running show.terminal show.idle show.unknown_session \
show.pinned_slot_unavailable show.url_rejected show.content_unavailable show.claim_conflict \
show.request_binding_missing show.provider_blocked show.release_failed show.lock_contended";
const RESUME_RESULTS: &str = "resume.running resume.terminal_success \
resume.terminal_optional_zero resume.unknown_session resume.pinned_slot_unavailable \
resume.url_rejected resume.content_unavailable resume.claim_conflict \
resume.output_publish_failed resume.request_binding_missing resume.provider_blocked \
resume.poll_failed resume.artifact_required_failed resume.release_failed resume.lock_contended";
const DOWNLOAD_RESULTS: &str = "download.completed download.optional_zero \
download.unknown_session download.pinned_slot_unavailable download.url_rejected \
download.claim_conflict download.content_unavailable download.controls_absent_required \
download.ambiguous_controls download.event_timeout download.integrity_failed \
download.provider_blocked download.release_failed download.lock_contended";
const RELEASE_RESULTS: &str = "release.allocatable release.cooldown_blocked \
release.already_released release.stop_skipped_owner_alive release.target_unknown \
release.fencing_mismatch release.takeover_unproven release.stop_failed release.cleanup_failed \
release.lock_contended";
const CLEANUP_RESULTS: &str = "cleanup.plan cleanup.applied cleanup.state_invalid \
cleanup.unsafe_path cleanup.partial_failure cleanup.lock_contended";
const STATE_REBUILD_RESULTS: &str = "state_rebuild.match state_rebuild.head_stale \
state_rebuild.snapshot_ignored state_rebuild.event_invalid state_rebuild.transition_invalid \
state_rebuild.digest_mismatch state_rebuild.lock_contended";
const ALLOCATE_RESULTS: &str = "allocate.dry_run_candidate allocate.pool_busy \
allocate.state_invalid allocate.lock_contended";

pub fn result_spec(command: &str, result_kind: &str) -> Option<ResultSpec> {
    let command_results = match command {
        "status" => STATUS_RESULTS,
        "preflight" => PREFLIGHT_RESULTS,
        "run" => RUN_RESULTS,
        "show" => SHOW_RESULTS,
        "resume" => RESUME_RESULTS,
        "download" => DOWNLOAD_RESULTS,
        "release" => RELEASE_RESULTS,
        "cleanup" => CLEANUP_RESULTS,
        "state-rebuild" => STATE_REBUILD_RESULTS,
        "allocate" => ALLOCATE_RESULTS,
        _ => return None,
    };
    if !command_results
        .split_ascii_whitespace()
        .any(|candidate| candidate == result_kind)
    {
        return None;
    }
    if result_kind.ends_with(".lock_contended") {
        return Some(ResultSpec {
            exit_code: 75,
            ok: false,
            reason_required: true,
            terminal: true,
        });
    }
    if matches!(
        result_kind,
        "run.running" | "run.queued_pool_busy" | "show.running" | "show.idle" | "resume.running"
    ) {
        return Some(ResultSpec {
            exit_code: 0,
            ok: true,
            reason_required: false,
            terminal: false,
        });
    }
    if matches!(
        result_kind,
        "status.ready"
            | "status.blocked"
            | "status.degraded"
            | "preflight.ready"
            | "preflight.model_correction_required"
            | "run.terminal_success"
            | "run.terminal_optional_zero"
            | "show.terminal"
            | "resume.terminal_success"
            | "resume.terminal_optional_zero"
            | "download.completed"
            | "download.optional_zero"
            | "release.allocatable"
            | "release.cooldown_blocked"
            | "release.already_released"
            | "release.stop_skipped_owner_alive"
            | "cleanup.plan"
            | "cleanup.applied"
            | "state_rebuild.match"
            | "state_rebuild.head_stale"
            | "state_rebuild.snapshot_ignored"
            | "allocate.dry_run_candidate"
            | "allocate.pool_busy"
    ) {
        return Some(ResultSpec {
            exit_code: 0,
            ok: true,
            reason_required: false,
            terminal: true,
        });
    }
    if matches!(
        result_kind,
        "preflight.login_required" | "preflight.subscription_required" | "preflight.provider_limit"
    ) {
        return Some(ResultSpec {
            exit_code: 0,
            ok: true,
            reason_required: true,
            terminal: true,
        });
    }
    Some(ResultSpec {
        exit_code: 70,
        ok: false,
        reason_required: true,
        terminal: true,
    })
}

impl LifecycleEnvelope {
    pub fn base(command: impl Into<String>, operation_id: impl Into<String>) -> Self {
        let command = command.into();
        Self {
            answer_path: None,
            answer_sha256: None,
            answer_size_bytes: None,
            answer_text: None,
            artifact_claims: Vec::new(),
            claim_id: None,
            cohort: None,
            command: command.clone(),
            conversation_url: None,
            evidence_root: None,
            event_ids: Vec::new(),
            lease_id: None,
            message: "lifecycle result".to_string(),
            ok: false,
            operation_id: operation_id.into(),
            reason: None,
            receipt_ids: Vec::new(),
            request_id: None,
            result_kind: format!("{command}.state_invalid"),
            retry: RetryDirective::default(),
            run_id: None,
            runtime_owner_id: None,
            schema: "gpt-webai.lifecycle.r13.v1".to_string(),
            session_id: None,
            slot_id: None,
            status: format!("{command}.state_invalid"),
            terminal: true,
        }
    }

    pub fn select(&mut self, result_kind: impl Into<String>, ok: bool, terminal: bool) {
        let result_kind = result_kind.into();
        self.status.clone_from(&result_kind);
        self.result_kind = result_kind;
        self.ok = ok;
        self.terminal = terminal;
    }

    pub fn select_matrix(&mut self, result_kind: impl Into<String>) -> Option<ResultSpec> {
        let result_kind = result_kind.into();
        let spec = result_spec(&self.command, &result_kind)?;
        self.select(result_kind, spec.ok, spec.terminal);
        Some(spec)
    }
}

impl CommandOutcome {
    pub fn select(
        command: impl Into<String>,
        operation_id: impl Into<String>,
        result_kind: impl Into<String>,
        message: impl Into<String>,
        reason: Option<String>,
    ) -> Result<Self, CommandOutcomeError> {
        let command = command.into();
        let operation_id = operation_id.into();
        let result_kind = result_kind.into();
        let message = message.into();
        if validate_operation_id(&operation_id).is_err() {
            return Err(CommandOutcomeError::InvalidOperationId);
        }
        if validate_non_empty_text(&message).is_err() {
            return Err(CommandOutcomeError::InvalidMessage);
        }
        let spec = result_spec(&command, &result_kind).ok_or_else(|| {
            CommandOutcomeError::UnknownResult {
                command: command.clone(),
                result_kind: result_kind.clone(),
            }
        })?;
        if spec.reason_required != reason.is_some() {
            return Err(CommandOutcomeError::InvalidReasonPresence);
        }
        let mut envelope = LifecycleEnvelope::base(command, operation_id);
        envelope
            .select_matrix(result_kind)
            .ok_or_else(|| CommandOutcomeError::UnknownResult {
                command: envelope.command.clone(),
                result_kind: envelope.result_kind.clone(),
            })?;
        envelope.message = message;
        envelope.reason = reason;
        Ok(Self {
            envelope,
            exit_code: spec.exit_code,
        })
    }
}
