use std::path::Path;
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::contracts::browser::EvidenceRef;
use crate::contracts::ids::{derive_runtime_incarnation_id, sha256_hex};
use crate::provider_runner::R13ProviderCommand;
use crate::runtime::control::{RuntimeReleaseMode, RuntimeStartMode};
use crate::runtime::docker_control::{
    compose_recreate, compose_stop, inspect_container, DockerControlError, RecreateInput,
};
use crate::runtime::ownership::{
    derive_runtime_owner_id, generate_incarnation_nonce, OwnershipError, RuntimeIdentityError,
};
use crate::runtime::{
    write_runtime_start_evidence, write_runtime_stop_evidence, RuntimeEvidenceError,
    RuntimeReceiptLabels,
};
use crate::slots;

#[derive(Clone, Debug)]
pub struct AcquiredRuntime {
    pub owner_id: String,
    pub owner_generation: u16,
    pub runtime_incarnation_id: String,
    pub docker_status: String,
    pub start_receipt: EvidenceRef,
    fake_inspect: Option<FakeInspectIdentity>,
}

#[derive(Clone, Debug)]
struct FakeInspectIdentity {
    container_id: String,
    container_name: String,
    started_at: String,
}

#[derive(Debug, Error)]
pub enum SessionRuntimeR13Error {
    #[error("slot not found for R13 session operation: {0}")]
    SlotMissing(String),
    #[error("runtime identity failed: {0}")]
    Identity(#[from] RuntimeIdentityError),
    #[error("runtime owner failed: {0}")]
    Owner(#[from] OwnershipError),
    #[error("runtime evidence failed: {0}")]
    Evidence(#[from] RuntimeEvidenceError),
    #[error("Docker runtime control failed: {0}")]
    Docker(#[from] DockerControlError),
    #[error("runtime identity derivation failed: {0}")]
    Id(#[from] crate::contracts::ids::IdError),
    #[error("runtime inspect serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn acquire_runtime(
    config: &SupervisorConfig,
    slot_id: &str,
    operation_id: &str,
    command: &R13ProviderCommand,
    mode: &RuntimeStartMode,
    captured_at_ms: u64,
) -> Result<AcquiredRuntime, SessionRuntimeR13Error> {
    let generation = 1;
    let nonce = generate_incarnation_nonce()?;
    let incarnation = derive_runtime_incarnation_id(slot_id, &nonce)?;
    let owner_id = derive_runtime_owner_id(slot_id, operation_id, generation)?;
    let labels = RuntimeReceiptLabels {
        owner_id: owner_id.clone(),
        owner_generation: generation,
        runtime_incarnation_id: incarnation.clone(),
    };
    let slot = slots::inventory(config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == slot_id)
        .ok_or_else(|| SessionRuntimeR13Error::SlotMissing(slot_id.to_string()))?;

    let (inspect, fake_inspect) = match mode {
        RuntimeStartMode::Disabled => {
            let identity = FakeInspectIdentity {
                container_id: sha256_hex(
                    format!("fake-runtime:{slot_id}:{operation_id}:{incarnation}").as_bytes(),
                ),
                container_name: format!("/{}", slot.container),
                started_at: format!("fake-{captured_at_ms}"),
            };
            (
                fake_inspect_bytes(&identity, &labels, "running", None, None)?,
                Some(identity),
            )
        }
        RuntimeStartMode::StartRuntime {
            docker_bin,
            timeout,
        } => {
            prepare_compose_bind_sources(config, slot_id)?;
            compose_recreate(RecreateInput {
                docker_bin,
                state_root: &config.state_root,
                slot_id,
                owner_id: &owner_id,
                owner_generation: generation,
                runtime_incarnation_id: &incarnation,
                timeout: *timeout,
            })?;
            (
                inspect_container(docker_bin, &slot.container, *timeout)?,
                None,
            )
        }
    };
    let start_receipt = write_runtime_start_evidence(
        &config.state_root,
        &command.paths.operation_host_dir,
        slot_id,
        &labels,
        &inspect,
        captured_at_ms,
    )?;
    Ok(AcquiredRuntime {
        owner_id,
        owner_generation: generation,
        runtime_incarnation_id: incarnation,
        docker_status: "running".to_string(),
        start_receipt,
        fake_inspect,
    })
}

pub fn stop_runtime(
    config: &SupervisorConfig,
    slot_id: &str,
    evidence_root: &Path,
    acquired: &AcquiredRuntime,
    mode: &RuntimeReleaseMode,
    captured_at_ms: u64,
) -> Result<EvidenceRef, SessionRuntimeR13Error> {
    let inspect = match (mode, acquired.fake_inspect.as_ref()) {
        (RuntimeReleaseMode::LockOnly, Some(identity)) => fake_inspect_bytes(
            identity,
            &RuntimeReceiptLabels {
                owner_id: acquired.owner_id.clone(),
                owner_generation: acquired.owner_generation,
                runtime_incarnation_id: acquired.runtime_incarnation_id.clone(),
            },
            "exited",
            Some(format!("fake-{captured_at_ms}")),
            Some(0),
        )?,
        (
            RuntimeReleaseMode::StopRuntime {
                docker_bin,
                timeout,
            },
            None,
        ) => {
            let service = slots::inventory(config)
                .into_iter()
                .find(|slot| slot.slot_id.0 == slot_id)
                .ok_or_else(|| SessionRuntimeR13Error::SlotMissing(slot_id.to_string()))?
                .container;
            let before = inspect_container(docker_bin, &service, *timeout)?;
            verify_runtime_labels(
                &before,
                &RuntimeReceiptLabels {
                    owner_id: acquired.owner_id.clone(),
                    owner_generation: acquired.owner_generation,
                    runtime_incarnation_id: acquired.runtime_incarnation_id.clone(),
                },
            )?;
            compose_stop(docker_bin, slot_id, *timeout)?;
            inspect_container(docker_bin, &service, *timeout)?
        }
        _ => return Err(SessionRuntimeR13Error::SlotMissing(slot_id.to_string())),
    };
    Ok(write_runtime_stop_evidence(
        &config.state_root,
        evidence_root,
        slot_id,
        &inspect,
        captured_at_ms,
    )?)
}

pub fn stop_owned_runtime(
    config: &SupervisorConfig,
    slot_id: &str,
    evidence_root: &Path,
    docker_bin: &Path,
    timeout: Duration,
    captured_at_ms: u64,
    expected_labels: &RuntimeReceiptLabels,
) -> Result<EvidenceRef, SessionRuntimeR13Error> {
    let before = observe_owned_runtime(
        config,
        slot_id,
        docker_bin,
        timeout,
        captured_at_ms,
        expected_labels,
    )?;
    verify_runtime_labels(&before, expected_labels)?;
    let inspect = if config.slot_mode == "fake" {
        fake_owned_inspect_bytes(slot_id, expected_labels, "exited", captured_at_ms)?
    } else {
        let service = slots::inventory(config)
            .into_iter()
            .find(|slot| slot.slot_id.0 == slot_id)
            .ok_or_else(|| SessionRuntimeR13Error::SlotMissing(slot_id.to_string()))?
            .container;
        compose_stop(docker_bin, slot_id, timeout)?;
        inspect_container(docker_bin, &service, timeout)?
    };
    Ok(write_runtime_stop_evidence(
        &config.state_root,
        evidence_root,
        slot_id,
        &inspect,
        captured_at_ms,
    )?)
}

pub fn observe_owned_runtime(
    config: &SupervisorConfig,
    slot_id: &str,
    docker_bin: &Path,
    timeout: Duration,
    captured_at_ms: u64,
    expected_labels: &RuntimeReceiptLabels,
) -> Result<Vec<u8>, SessionRuntimeR13Error> {
    if config.slot_mode == "fake" {
        return Ok(fake_owned_inspect_bytes(
            slot_id,
            expected_labels,
            "running",
            captured_at_ms,
        )?);
    }
    let service = slots::inventory(config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == slot_id)
        .ok_or_else(|| SessionRuntimeR13Error::SlotMissing(slot_id.to_string()))?
        .container;
    Ok(inspect_container(docker_bin, &service, timeout)?)
}

fn verify_runtime_labels(
    inspect_bytes: &[u8],
    expected: &RuntimeReceiptLabels,
) -> Result<(), SessionRuntimeR13Error> {
    let inspect = crate::runtime::parse_docker_inspect(inspect_bytes)?;
    if inspect.label_owner_id.as_deref() != Some(expected.owner_id.as_str())
        || inspect.label_generation != Some(expected.owner_generation)
        || inspect.label_incarnation.as_deref() != Some(expected.runtime_incarnation_id.as_str())
    {
        return Err(SessionRuntimeR13Error::Evidence(
            RuntimeEvidenceError::Invalid("stop labels"),
        ));
    }
    Ok(())
}

pub fn runtime_stop_timeout(mode: &RuntimeReleaseMode) -> Duration {
    match mode {
        RuntimeReleaseMode::LockOnly => Duration::from_millis(30_000),
        RuntimeReleaseMode::StopRuntime { timeout, .. } => *timeout,
    }
}

fn prepare_compose_bind_sources(
    config: &SupervisorConfig,
    slot_id: &str,
) -> Result<(), SessionRuntimeR13Error> {
    let slot_root = config.state_root.join("slots").join(slot_id);
    for path in [
        slot_root.join("state"),
        slot_root.join("attachments"),
        slot_root.join("prompts"),
        config.state_root.join("artifacts"),
    ] {
        crate::provider_runner::create_private_directory(&config.state_root, &path)
            .map_err(DockerControlError::Io)?;
    }
    Ok(())
}

fn fake_inspect_bytes(
    identity: &FakeInspectIdentity,
    labels: &RuntimeReceiptLabels,
    status: &str,
    finished_at: Option<String>,
    exit_code: Option<i64>,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!([{
        "Id": identity.container_id,
        "Name": identity.container_name,
        "Config": {"Labels": {
            "pr72.gpt-webai.owner-id": labels.owner_id,
            "pr72.gpt-webai.owner-generation": labels.owner_generation.to_string(),
            "pr72.gpt-webai.runtime-incarnation": labels.runtime_incarnation_id
        }},
        "State": {
            "Status": status,
            "StartedAt": identity.started_at,
            "FinishedAt": finished_at,
            "ExitCode": exit_code
        }
    }]))
}

fn fake_owned_inspect_bytes(
    slot_id: &str,
    labels: &RuntimeReceiptLabels,
    status: &str,
    captured_at_ms: u64,
) -> Result<Vec<u8>, serde_json::Error> {
    let identity = FakeInspectIdentity {
        container_id: sha256_hex(
            format!(
                "fake-owned-runtime:{slot_id}:{}:{}",
                labels.owner_id, labels.runtime_incarnation_id
            )
            .as_bytes(),
        ),
        container_name: format!("/gpt-webai-{slot_id}"),
        started_at: format!("fake-owner-generation-{}", labels.owner_generation),
    };
    let (finished_at, exit_code) = if status == "exited" {
        (Some(format!("fake-{captured_at_ms}")), Some(0))
    } else {
        (None, None)
    };
    fake_inspect_bytes(&identity, labels, status, finished_at, exit_code)
}
