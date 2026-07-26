use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde_json::json;
use thiserror::Error;

use crate::artifact_claims::baseline::{ArtifactBaseline, ArtifactBaselineEntry};
use crate::artifact_claims::recovery::{ArtifactClaim, ClaimOutcome};
use crate::artifact_claims::{ArtifactClaimError, ArtifactExpectation};
use crate::claims::{derived_id, CasError};
use crate::config::{now_ms, SupervisorConfig};
use crate::contracts::browser::SessionEcho;
use crate::contracts::cli::ArtifactClaimSummary;
use crate::contracts::events::{AggregateKind, EventEnvelope, EventType};
use crate::contracts::ids::{h256, validate_operation_id};
use crate::contracts::provider::ProviderIdentity;
use crate::journal::canonical::canonical_bytes;
use crate::provider_runner::{ProviderExecution, ProviderRunnerError, R13ProviderCommandContext};
use crate::sessions::SessionRecord;

use super::journal::{NewEvent, SessionJournal, SessionJournalError};
use super::provider::{
    build_artifact_click_request, build_artifact_discover_request, invoke_artifact_click,
    invoke_artifact_discover, ProviderLimits, RebindProviderError,
};

#[derive(Debug, Error)]
pub enum ArtifactPipelineError {
    #[error("artifact pipeline journal failed: {0}")]
    Journal(#[from] SessionJournalError),
    #[error("artifact pipeline provider command failed: {0}")]
    ProviderCommand(#[from] ProviderRunnerError),
    #[error("artifact pipeline provider failed: {0}")]
    Provider(#[from] RebindProviderError),
    #[error("artifact pipeline claim failed: {0}")]
    Claim(#[from] ArtifactClaimError),
    #[error("artifact pipeline identifier failed: {0}")]
    Id(#[from] CasError),
    #[error("artifact pipeline io failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact pipeline json failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("artifact pipeline contract failed: {0}")]
    Contract(&'static str),
}

pub struct ArtifactPipelineInput<'a> {
    pub config: &'a SupervisorConfig,
    pub journal: &'a mut SessionJournal,
    pub provider_execution: &'a ProviderExecution,
    pub provider_limits: ProviderLimits,
    pub operation_id: &'a str,
    pub request_key: &'a str,
    pub request_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub record: &'a SessionRecord,
    pub expected: &'a SessionEcho,
    pub source_event: &'a EventEnvelope,
    pub terminal_assistant_turn_id: &'a str,
    pub expectation: &'a str,
}

pub struct ArtifactPipelineOutput {
    pub terminal_event: EventEnvelope,
    pub summary: ArtifactClaimSummary,
    pub receipt_ids: Vec<String>,
    pub optional_zero: bool,
    pub failure_reason: Option<String>,
}

pub fn recover_artifacts(
    input: ArtifactPipelineInput<'_>,
) -> Result<ArtifactPipelineOutput, ArtifactPipelineError> {
    let expectation = parse_expectation(input.expectation)?;
    let claim_id = derived_id(
        "artifact_claim_",
        &json!([
            "pr72.artifact-claim.r13.v1",
            input.request_id.unwrap_or(&input.record.session_id),
            input.record.session_id,
            input.operation_id
        ]),
    )?;
    let mut claim = ArtifactClaim::establish(
        claim_id.clone(),
        input.record.session_id.clone(),
        input.terminal_assistant_turn_id.to_string(),
        expectation,
    )?;
    let established_at = now_ms();
    let established = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::ArtifactClaim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::ArtifactClaimEstablished,
        payload: json!({
            "artifactClaimId":claim_id,"sessionId":input.record.session_id,
            "requestId":input.request_id,"expectation":input.expectation,
            "terminalAssistantTurnId":input.terminal_assistant_turn_id,
            "establishedAtMs":established_at
        }),
        predecessor_event_id: None,
        source_event_ids: vec![input.source_event.event_id.clone()],
        created_at_ms: established_at,
    })?;

    let discover_id = child_operation_id(input.operation_id, "artifact-discover")?;
    let discover_command = input
        .provider_execution
        .r13_command(R13ProviderCommandContext {
            config: input.config,
            slot_id: &input.record.slot_id,
            request_key: input.request_key,
            operation_id: &discover_id,
        })?;
    let discovered = invoke_artifact_discover(
        &discover_command,
        &build_artifact_discover_request(
            identity(&input, discover_id),
            input.expected,
            &claim_id,
            input.terminal_assistant_turn_id,
            input.expectation,
        ),
        &input.config.state_root,
        input.provider_limits,
    )?;
    let mut receipt_ids = discovered.receipt_ids.clone();
    if discovered.data.failure_reason.as_deref() != discovered.provider_reason.as_deref() {
        return Err(ArtifactPipelineError::Contract(
            "artifact discover failure reason",
        ));
    }
    if !discovered.ok {
        let reason = artifact_reason(discovered.provider_reason.as_deref())?;
        claim.fail(reason)?;
        return finish_failure(
            input.journal,
            &claim_id,
            &established,
            reason,
            Some(discovered.receipt),
            receipt_ids,
            input.expectation,
        );
    }
    if discovered.data.observed_echo.as_ref() != Some(input.expected) {
        return Err(ArtifactPipelineError::Contract(
            "artifact discover session echo",
        ));
    }

    if discovered.data.controls.is_empty() {
        let proof = discovered
            .data
            .zero_control_proof
            .as_ref()
            .ok_or(ArtifactPipelineError::Contract("zeroControlProof"))?;
        if discovered.data.bottom_proof.as_ref() != Some(&proof.bottom_proof) {
            return Err(ArtifactPipelineError::Contract("zero bottom proof"));
        }
        claim.discover_zero(proof)?;
        let observed_at = now_ms();
        let absent = input.journal.append(NewEvent {
            aggregate_kind: AggregateKind::ArtifactClaim,
            aggregate_id: claim_id.clone(),
            event_type: EventType::ArtifactControlsAbsent,
            payload: json!({
                "artifactClaimId":claim_id,"zeroControlProof":proof,
                "providerReceipt":discovered.receipt,"observedAtMs":observed_at
            }),
            predecessor_event_id: Some(established.event_id.clone()),
            source_event_ids: vec![established.event_id.clone()],
            created_at_ms: observed_at,
        })?;
        return match claim.outcome() {
            ClaimOutcome::ZeroControlsOptionalSuccess => {
                let completed_at = now_ms();
                let completed = input.journal.append(NewEvent {
                    aggregate_kind: AggregateKind::ArtifactClaim,
                    aggregate_id: claim_id.clone(),
                    event_type: EventType::ArtifactClaimCompleted,
                    payload: json!({
                        "artifactClaimId":claim_id,"result":"zero_controls_optional_success",
                        "artifactCount":0,"manifestPath":null,"manifestSha256":null,
                        "completedAtMs":completed_at
                    }),
                    predecessor_event_id: Some(absent.event_id.clone()),
                    source_event_ids: vec![absent.event_id],
                    created_at_ms: completed_at,
                })?;
                Ok(success_output(
                    completed,
                    claim_id,
                    input.expectation,
                    Vec::new(),
                    receipt_ids,
                    true,
                ))
            }
            ClaimOutcome::Failed(reason) => finish_failure(
                input.journal,
                &claim_id,
                &absent,
                reason,
                Some(discovered.receipt),
                receipt_ids,
                input.expectation,
            ),
            _ => Err(ArtifactPipelineError::Contract("zero claim outcome")),
        };
    }

    let bottom = discovered
        .data
        .bottom_proof
        .as_ref()
        .ok_or(ArtifactPipelineError::Contract("artifact bottom proof"))?;
    if discovered.data.zero_control_proof.is_some() {
        return Err(ArtifactPipelineError::Contract("positive zeroControlProof"));
    }
    claim.discover_controls(discovered.data.controls.clone(), bottom)?;
    let controls_at = now_ms();
    let mut predecessor = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::ArtifactClaim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::ArtifactControlsDiscovered,
        payload: json!({
            "artifactClaimId":claim_id,"controls":discovered.data.controls,
            "controlCount":discovered.data.controls.len(),"bottomProof":bottom,
            "providerReceipt":discovered.receipt,"discoveredAtMs":controls_at
        }),
        predecessor_event_id: Some(established.event_id.clone()),
        source_event_ids: vec![established.event_id],
        created_at_ms: controls_at,
    })?;
    let mut completed_events = Vec::new();
    let mut manifest_members = Vec::new();
    let host_save_directory = format!("artifacts/{}/{claim_id}", input.request_key);
    for index in 0..discovered.data.controls.len() {
        let click_id = child_operation_id(input.operation_id, &format!("artifact-click-{index}"))?;
        let click_command = input
            .provider_execution
            .r13_command(R13ProviderCommandContext {
                config: input.config,
                slot_id: &input.record.slot_id,
                request_key: input.request_key,
                operation_id: &click_id,
            })?;
        let baseline = capture_baseline(&input.config.state_root, &host_save_directory)?;
        let consumed = claim
            .consume_next(click_id.clone(), baseline.clone())?
            .clone();
        let receipt_path = relative_path(
            &input.config.state_root,
            &click_command
                .paths
                .operation_host_dir
                .join("provider-receipt.json"),
        )?;
        let consumed_at = now_ms();
        let consumed_event = input.journal.append(NewEvent {
            aggregate_kind: AggregateKind::ArtifactClaim,
            aggregate_id: claim_id.clone(),
            event_type: EventType::ArtifactDownloadAttemptConsumed,
            payload: json!({
                "artifactClaimId":claim_id,"attemptId":click_id,
                "controlIndex":consumed.control_index,"controlId":consumed.control.control_id,
                "artifactBaseline":baseline,"providerReceiptPath":receipt_path,
                "hostSaveDirectory":host_save_directory,"clickBudget":1,
                "attemptConsumedAtMs":consumed_at
            }),
            predecessor_event_id: Some(predecessor.event_id.clone()),
            source_event_ids: vec![predecessor.event_id.clone()],
            created_at_ms: consumed_at,
        })?;
        let clicked = invoke_artifact_click(
            &click_command,
            &build_artifact_click_request(
                identity(&input, click_id),
                input.expected,
                &claim_id,
                input.terminal_assistant_turn_id,
                &consumed.control,
                &baseline,
                consumed.control_index,
                &host_save_directory,
            ),
            &input.config.state_root,
            input.provider_limits,
        )?;
        receipt_ids.extend(clicked.receipt_ids.clone());
        if clicked.data.failure_reason.as_deref() != clicked.provider_reason.as_deref() {
            return Err(ArtifactPipelineError::Contract(
                "artifact click failure reason",
            ));
        }
        if !clicked.ok {
            let reason = artifact_reason(clicked.provider_reason.as_deref())?;
            claim.fail(reason)?;
            return finish_failure(
                input.journal,
                &claim_id,
                &consumed_event,
                reason,
                Some(clicked.receipt),
                receipt_ids,
                input.expectation,
            );
        }
        if clicked.data.observed_echo.as_ref() != Some(input.expected) {
            return Err(ArtifactPipelineError::Contract(
                "artifact click session echo",
            ));
        }
        let download = clicked
            .data
            .download_receipt
            .ok_or(ArtifactPipelineError::Contract("downloadReceipt"))?;
        claim.complete_consumed(input.expected, download.clone(), &input.config.state_root)?;
        let completed_at = now_ms();
        let completed = input.journal.append(NewEvent {
            aggregate_kind: AggregateKind::ArtifactClaim,
            aggregate_id: claim_id.clone(),
            event_type: EventType::ArtifactDownloadCompleted,
            payload: json!({
                "artifactClaimId":claim_id,"attemptId":consumed.attempt_id,
                "controlIndex":consumed.control_index,"artifactId":download.artifact_id,
                "downloadReceipt":download,"completedAtMs":completed_at
            }),
            predecessor_event_id: Some(consumed_event.event_id.clone()),
            source_event_ids: vec![consumed_event.event_id],
            created_at_ms: completed_at,
        })?;
        let receipt_id = clicked
            .receipt_ids
            .last()
            .ok_or(ArtifactPipelineError::Contract("artifact receipt id"))?;
        manifest_members.push(json!({
            "controlIndex":consumed.control_index,"artifactId":download.artifact_id,
            "controlId":consumed.control.control_id,"hostSavedRelPath":download.host_saved_rel_path,
            "sizeBytes":download.size_bytes,"sha256":download.sha256,
            "mediaType":download.media_type,"receiptId":receipt_id
        }));
        completed_events.push(completed.clone());
        predecessor = completed;
    }
    if !matches!(claim.outcome(), ClaimOutcome::Downloaded) {
        return Err(ArtifactPipelineError::Contract("download claim outcome"));
    }
    let (manifest_path, manifest_sha256) = write_manifest(
        &input.config.state_root,
        &host_save_directory,
        &claim_id,
        &manifest_members,
    )?;
    let completed_at = now_ms();
    let completed = input.journal.append(NewEvent {
        aggregate_kind: AggregateKind::ArtifactClaim,
        aggregate_id: claim_id.clone(),
        event_type: EventType::ArtifactClaimCompleted,
        payload: json!({
            "artifactClaimId":claim_id,"result":"downloaded",
            "artifactCount":manifest_members.len(),"manifestPath":manifest_path,
            "manifestSha256":manifest_sha256,"completedAtMs":completed_at
        }),
        predecessor_event_id: Some(predecessor.event_id),
        source_event_ids: completed_events
            .iter()
            .map(|event| event.event_id.clone())
            .collect(),
        created_at_ms: completed_at,
    })?;
    let artifact_ids = manifest_members
        .iter()
        .filter_map(|member| member["artifactId"].as_str().map(str::to_owned))
        .collect();
    Ok(success_output(
        completed,
        claim_id,
        input.expectation,
        artifact_ids,
        receipt_ids,
        false,
    ))
}

fn success_output(
    terminal_event: EventEnvelope,
    claim_id: String,
    expectation: &str,
    artifact_ids: Vec<String>,
    receipt_ids: Vec<String>,
    optional_zero: bool,
) -> ArtifactPipelineOutput {
    ArtifactPipelineOutput {
        terminal_event,
        summary: ArtifactClaimSummary {
            artifact_claim_id: claim_id,
            expectation: expectation.to_string(),
            status: "completed".to_string(),
            result: Some(
                if optional_zero {
                    "zero_controls_optional_success"
                } else {
                    "downloaded"
                }
                .to_string(),
            ),
            artifact_ids,
        },
        receipt_ids,
        optional_zero,
        failure_reason: None,
    }
}

fn finish_failure(
    journal: &mut SessionJournal,
    claim_id: &str,
    predecessor: &EventEnvelope,
    reason: &str,
    provider_receipt: Option<crate::contracts::browser::EvidenceRef>,
    receipt_ids: Vec<String>,
    expectation: &str,
) -> Result<ArtifactPipelineOutput, ArtifactPipelineError> {
    let failed_at = now_ms();
    let failed = journal.append(NewEvent {
        aggregate_kind: AggregateKind::ArtifactClaim,
        aggregate_id: claim_id.to_string(),
        event_type: EventType::ArtifactClaimFailed,
        payload: json!({
            "artifactClaimId":claim_id,"reason":reason,"failedControlIndex":null,
            "providerReceipt":provider_receipt,"failedAtMs":failed_at
        }),
        predecessor_event_id: Some(predecessor.event_id.clone()),
        source_event_ids: Vec::new(),
        created_at_ms: failed_at,
    })?;
    Ok(ArtifactPipelineOutput {
        terminal_event: failed,
        summary: ArtifactClaimSummary {
            artifact_claim_id: claim_id.to_string(),
            expectation: expectation.to_string(),
            status: "failed".to_string(),
            result: None,
            artifact_ids: Vec::new(),
        },
        receipt_ids,
        optional_zero: false,
        failure_reason: Some(reason.to_string()),
    })
}

fn parse_expectation(value: &str) -> Result<ArtifactExpectation, ArtifactPipelineError> {
    match value {
        "none" => Ok(ArtifactExpectation::None),
        "optional" => Ok(ArtifactExpectation::Optional),
        "required" => Ok(ArtifactExpectation::Required),
        "claimed" => Ok(ArtifactExpectation::Claimed),
        _ => Err(ArtifactPipelineError::Contract("artifact expectation")),
    }
}

fn artifact_reason(value: Option<&str>) -> Result<&'static str, ArtifactPipelineError> {
    match value {
        Some("artifact.required_zero") => Ok("artifact.required_zero"),
        Some("artifact.controls_ambiguous") => Ok("artifact.controls_ambiguous"),
        Some("artifact.bottom_unverified") => Ok("artifact.bottom_unverified"),
        Some("artifact.download_timeout") => Ok("artifact.download_timeout"),
        Some("artifact.event_unrecoverable") => Ok("artifact.event_unrecoverable"),
        Some("artifact.integrity_failed") => Ok("artifact.integrity_failed"),
        Some("artifact.path_unsafe") => Ok("artifact.path_unsafe"),
        _ => Err(ArtifactPipelineError::Contract("artifact failure reason")),
    }
}

fn identity(input: &ArtifactPipelineInput<'_>, operation_id: String) -> ProviderIdentity {
    ProviderIdentity {
        cohort: Some(input.record.cohort.clone()),
        operation_id,
        request_id: input.request_id.map(str::to_string),
        run_id: input.run_id.map(str::to_string),
        session_id: Some(input.record.session_id.clone()),
        slot_id: input.record.slot_id.clone(),
    }
}

fn child_operation_id(parent: &str, suffix: &str) -> Result<String, ArtifactPipelineError> {
    let value = format!("{parent}.{suffix}");
    validate_operation_id(&value)
        .map_err(|_| ArtifactPipelineError::Contract("artifact operationId"))?;
    Ok(value)
}

fn capture_baseline(
    root: &Path,
    directory: &str,
) -> Result<ArtifactBaseline, ArtifactPipelineError> {
    let absolute = root.join(directory);
    crate::provider_runner::create_private_directory(root, &absolute)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&absolute)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
            return Err(ArtifactPipelineError::Contract("artifact baseline entry"));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ArtifactPipelineError::Contract("artifact baseline filename"))?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(entry.path())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        entries.push(ArtifactBaselineEntry {
            rel_path: name,
            size_bytes: bytes.len() as u64,
            sha256: h256(bytes),
        });
    }
    entries.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    let captured_at_ms = now_ms();
    let baseline_sha256 = h256(canonical_bytes(&entries)?);
    let baseline = ArtifactBaseline {
        directory: directory.to_string(),
        entries,
        captured_at_ms,
        baseline_sha256,
    };
    baseline.validate()?;
    Ok(baseline)
}

fn write_manifest(
    root: &Path,
    directory: &str,
    claim_id: &str,
    members: &[serde_json::Value],
) -> Result<(String, String), ArtifactPipelineError> {
    let created_at_ms = now_ms();
    let bytes = canonical_bytes(&json!({
        "schemaVersion":"pr72.artifact-manifest.r13.v1",
        "artifactClaimId":claim_id,"members":members,"createdAtMs":created_at_ms
    }))?;
    let relative = format!("{directory}/MANIFEST.json");
    let target = root.join(&relative);
    let parent = target
        .parent()
        .ok_or(ArtifactPipelineError::Contract("manifest parent"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    if fs::read(&target)? != bytes {
        return Err(ArtifactPipelineError::Contract("manifest verification"));
    }
    Ok((relative, h256(bytes)))
}

fn relative_path(root: &Path, path: &Path) -> Result<String, ArtifactPipelineError> {
    path.strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or(ArtifactPipelineError::Contract("state-relative path"))
}
