use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::contracts::browser::{EvidenceMediaType, EvidenceRef};
use crate::contracts::ids::{
    h256, validate_generation, validate_owner_id, validate_runtime_incarnation_id,
    validate_safe_rel_path, validate_slot_id, validate_timestamp_ms,
};
use crate::journal::canonical::canonical_bytes;
use crate::provider_client::validate_provider_envelope;
use crate::provider_runner::ProviderExecution;
use crate::slots::SlotConfig;

use super::probe::{DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe};

const OWNER_ID_LABEL: &str = "pr72.gpt-webai.owner-id";
const OWNER_GENERATION_LABEL: &str = "pr72.gpt-webai.owner-generation";
const RUNTIME_INCARNATION_LABEL: &str = "pr72.gpt-webai.runtime-incarnation";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerInspectRecord {
    pub container_id: String,
    pub container_name: String,
    pub docker_status: String,
    pub container_started_at: String,
    pub container_finished_at: Option<String>,
    pub exit_code: Option<i64>,
    pub label_owner_id: Option<String>,
    pub label_generation: Option<u16>,
    pub label_incarnation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeReceiptLabels {
    pub owner_id: String,
    pub owner_generation: u16,
    pub runtime_incarnation_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeStartReceipt {
    pub schema_version: String,
    pub slot_id: String,
    pub container_id: String,
    pub container_name: String,
    pub docker_status: String,
    pub container_started_at: String,
    pub labels: RuntimeReceiptLabels,
    pub inspect_sha256: String,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeStopReceipt {
    pub schema_version: String,
    pub slot_id: String,
    pub container_id: String,
    pub container_name: String,
    pub docker_status: String,
    pub container_started_at: String,
    pub container_finished_at: Option<String>,
    pub exit_code: Option<i64>,
    pub inspect_sha256: String,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeAdoptionReceipt {
    pub schema_version: String,
    pub slot_id: String,
    pub container_id: String,
    pub observed_docker_status: String,
    pub container_label_owner_id: Option<String>,
    pub container_label_generation: Option<u16>,
    pub container_label_incarnation: Option<String>,
    pub inspect_sha256: String,
    pub captured_at_ms: u64,
}

#[derive(Debug, Error)]
pub enum RuntimeEvidenceError {
    #[error("docker inspect contract invalid: {0}")]
    Invalid(&'static str),
    #[error("runtime evidence io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime evidence json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn parse_docker_inspect(bytes: &[u8]) -> Result<DockerInspectRecord, RuntimeEvidenceError> {
    let value: Value = serde_json::from_slice(bytes)?;
    let item = value
        .as_array()
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .ok_or(RuntimeEvidenceError::Invalid("root"))?;
    let state = item
        .get("State")
        .and_then(Value::as_object)
        .ok_or(RuntimeEvidenceError::Invalid("State"))?;
    let labels = item
        .get("Config")
        .and_then(Value::as_object)
        .and_then(|config| config.get("Labels"))
        .and_then(Value::as_object);
    let label = |key: &str| {
        labels
            .and_then(|items| items.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let label_generation = label(OWNER_GENERATION_LABEL)
        .map(|value| value.parse::<u16>())
        .transpose()
        .map_err(|_| RuntimeEvidenceError::Invalid("owner generation label"))?;
    Ok(DockerInspectRecord {
        container_id: required_string(item.get("Id"), "Id")?,
        container_name: required_string(item.get("Name"), "Name")?,
        docker_status: canonical_docker_status(required_string(
            state.get("Status"),
            "State.Status",
        )?),
        container_started_at: required_string(state.get("StartedAt"), "State.StartedAt")?,
        container_finished_at: optional_nonempty_string(state.get("FinishedAt"))?,
        exit_code: state.get("ExitCode").and_then(Value::as_i64),
        label_owner_id: label(OWNER_ID_LABEL),
        label_generation,
        label_incarnation: label(RUNTIME_INCARNATION_LABEL),
    })
}

pub fn write_runtime_start_evidence(
    state_root: &Path,
    evidence_root: &Path,
    slot_id: &str,
    expected_labels: &RuntimeReceiptLabels,
    inspect_bytes: &[u8],
    captured_at_ms: u64,
) -> Result<EvidenceRef, RuntimeEvidenceError> {
    validate_evidence_root(state_root, evidence_root)?;
    let inspect = validate_inspect(slot_id, inspect_bytes, captured_at_ms)?;
    let labels = RuntimeReceiptLabels {
        owner_id: inspect
            .label_owner_id
            .clone()
            .ok_or(RuntimeEvidenceError::Invalid("owner label"))?,
        owner_generation: inspect
            .label_generation
            .ok_or(RuntimeEvidenceError::Invalid("generation label"))?,
        runtime_incarnation_id: inspect
            .label_incarnation
            .clone()
            .ok_or(RuntimeEvidenceError::Invalid("incarnation label"))?,
    };
    if validate_owner_id(&labels.owner_id).is_err()
        || validate_generation(labels.owner_generation).is_err()
        || validate_runtime_incarnation_id(&labels.runtime_incarnation_id).is_err()
        || labels != *expected_labels
    {
        return Err(RuntimeEvidenceError::Invalid("start labels"));
    }
    let inspect_sha256 = h256(inspect_bytes);
    write_inspect(state_root, evidence_root, inspect_bytes)?;
    let receipt = RuntimeStartReceipt {
        schema_version: "pr72.runtime-start-receipt.r13.v1".to_string(),
        slot_id: slot_id.to_string(),
        container_id: inspect.container_id,
        container_name: inspect.container_name,
        docker_status: inspect.docker_status,
        container_started_at: inspect.container_started_at,
        labels,
        inspect_sha256,
        captured_at_ms,
    };
    write_receipt_ref(
        state_root,
        evidence_root,
        "runtime-start.receipt.json",
        &receipt,
    )
}

pub fn write_runtime_stop_evidence(
    state_root: &Path,
    evidence_root: &Path,
    slot_id: &str,
    inspect_bytes: &[u8],
    captured_at_ms: u64,
) -> Result<EvidenceRef, RuntimeEvidenceError> {
    validate_evidence_root(state_root, evidence_root)?;
    let inspect = validate_inspect(slot_id, inspect_bytes, captured_at_ms)?;
    let inspect_sha256 = h256(inspect_bytes);
    write_inspect(state_root, evidence_root, inspect_bytes)?;
    let receipt = RuntimeStopReceipt {
        schema_version: "pr72.runtime-stop-receipt.r13.v1".to_string(),
        slot_id: slot_id.to_string(),
        container_id: inspect.container_id,
        container_name: inspect.container_name,
        docker_status: inspect.docker_status,
        container_started_at: inspect.container_started_at,
        container_finished_at: inspect.container_finished_at,
        exit_code: inspect.exit_code,
        inspect_sha256,
        captured_at_ms,
    };
    write_receipt_ref(
        state_root,
        evidence_root,
        "runtime-stop.receipt.json",
        &receipt,
    )
}

pub fn write_runtime_adoption_evidence(
    state_root: &Path,
    evidence_root: &Path,
    slot_id: &str,
    inspect_bytes: &[u8],
    captured_at_ms: u64,
) -> Result<EvidenceRef, RuntimeEvidenceError> {
    validate_evidence_root(state_root, evidence_root)?;
    let inspect = validate_inspect(slot_id, inspect_bytes, captured_at_ms)?;
    if inspect.label_incarnation.is_none()
        || inspect
            .label_owner_id
            .as_deref()
            .is_some_and(|value| validate_owner_id(value).is_err())
        || inspect
            .label_incarnation
            .as_deref()
            .is_some_and(|value| validate_runtime_incarnation_id(value).is_err())
        || inspect
            .label_generation
            .is_some_and(|value| validate_generation(value).is_err())
    {
        return Err(RuntimeEvidenceError::Invalid("adoption labels"));
    }
    let inspect_sha256 = h256(inspect_bytes);
    write_inspect(state_root, evidence_root, inspect_bytes)?;
    let receipt = RuntimeAdoptionReceipt {
        schema_version: "pr72.runtime-adoption-receipt.r13.v1".to_string(),
        slot_id: slot_id.to_string(),
        container_id: inspect.container_id,
        observed_docker_status: inspect.docker_status,
        container_label_owner_id: inspect.label_owner_id,
        container_label_generation: inspect.label_generation,
        container_label_incarnation: inspect.label_incarnation,
        inspect_sha256,
        captured_at_ms,
    };
    write_receipt_ref(
        state_root,
        evidence_root,
        "runtime-adoption.receipt.json",
        &receipt,
    )
}

fn validate_inspect(
    slot_id: &str,
    bytes: &[u8],
    captured_at_ms: u64,
) -> Result<DockerInspectRecord, RuntimeEvidenceError> {
    if validate_slot_id(slot_id).is_err() || validate_timestamp_ms(captured_at_ms).is_err() {
        return Err(RuntimeEvidenceError::Invalid("identity/time"));
    }
    let inspect = parse_docker_inspect(bytes)?;
    if inspect.container_id.len() != 64
        || !inspect
            .container_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RuntimeEvidenceError::Invalid("container identity"));
    }
    Ok(inspect)
}

fn validate_evidence_root(
    state_root: &Path,
    evidence_root: &Path,
) -> Result<(), RuntimeEvidenceError> {
    if !state_root.is_absolute() || !evidence_root.is_absolute() {
        return Err(RuntimeEvidenceError::Invalid("evidence root"));
    }
    let relative = evidence_root
        .strip_prefix(state_root)
        .map_err(|_| RuntimeEvidenceError::Invalid("evidence root"))?
        .to_str()
        .ok_or(RuntimeEvidenceError::Invalid("evidence path"))?
        .replace('\\', "/");
    validate_safe_rel_path(&relative)
        .map_err(|_| RuntimeEvidenceError::Invalid("evidence path"))?;
    Ok(())
}

fn write_inspect(
    state_root: &Path,
    evidence_root: &Path,
    bytes: &[u8],
) -> Result<(), RuntimeEvidenceError> {
    if bytes.is_empty() {
        return Err(RuntimeEvidenceError::Invalid("empty inspect"));
    }
    write_immutable(
        state_root,
        &evidence_root.join("docker-inspect.json"),
        bytes,
    )
}

fn write_receipt_ref<T: Serialize>(
    state_root: &Path,
    evidence_root: &Path,
    name: &str,
    receipt: &T,
) -> Result<EvidenceRef, RuntimeEvidenceError> {
    let bytes = canonical_bytes(receipt)
        .map_err(|_| RuntimeEvidenceError::Invalid("receipt serialization"))?;
    let path = evidence_root.join(name);
    write_immutable(state_root, &path, &bytes)?;
    let relative = path
        .strip_prefix(state_root)
        .map_err(|_| RuntimeEvidenceError::Invalid("evidence root"))?
        .to_str()
        .ok_or(RuntimeEvidenceError::Invalid("evidence path"))?
        .replace('\\', "/");
    Ok(EvidenceRef {
        path: relative,
        sha256: h256(&bytes),
        size_bytes: u64::try_from(bytes.len())
            .map_err(|_| RuntimeEvidenceError::Invalid("receipt size"))?,
        media_type: EvidenceMediaType::Json,
    })
}

fn write_immutable(
    state_root: &Path,
    path: &Path,
    bytes: &[u8],
) -> Result<(), RuntimeEvidenceError> {
    let parent = path
        .parent()
        .ok_or(RuntimeEvidenceError::Invalid("evidence parent"))?;
    crate::provider_runner::create_private_directory(state_root, parent)?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            File::open(parent)?.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (fs::read(path)?
            == bytes)
            .then_some(())
            .ok_or(RuntimeEvidenceError::Invalid("immutable collision")),
        Err(error) => Err(error.into()),
    }
}

fn required_string(
    value: Option<&Value>,
    field: &'static str,
) -> Result<String, RuntimeEvidenceError> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && !text.contains('\0'))
        .map(str::to_string)
        .ok_or(RuntimeEvidenceError::Invalid(field))
}

fn optional_nonempty_string(value: Option<&Value>) -> Result<Option<String>, RuntimeEvidenceError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() && !text.contains('\0') => {
            Ok(Some(text.clone()))
        }
        _ => Err(RuntimeEvidenceError::Invalid("optional string")),
    }
}

fn canonical_docker_status(value: String) -> String {
    match value.as_str() {
        "running" | "exited" | "starting" | "stopping" | "unknown" => value,
        _ => "unknown".to_string(),
    }
}

#[derive(Clone, Debug)]
pub struct DockerRuntime {
    docker_bin: PathBuf,
    slot_mode: String,
    state_root: std::path::PathBuf,
    status_provider_check: bool,
    provider_status_timeout: Duration,
}

impl DockerRuntime {
    pub fn new(config: &SupervisorConfig) -> Self {
        Self::with_docker_bin(config, PathBuf::from("docker"))
    }

    pub fn with_docker_bin(config: &SupervisorConfig, docker_bin: PathBuf) -> Self {
        Self {
            docker_bin,
            slot_mode: config.slot_mode.clone(),
            state_root: config.state_root.clone(),
            status_provider_check: config.status_provider_check,
            provider_status_timeout: Duration::from_millis(config.provider_status_timeout_ms),
        }
    }
}

pub fn docker_runtime_for_provider(
    config: &SupervisorConfig,
    provider_execution: &ProviderExecution,
) -> DockerRuntime {
    provider_execution
        .docker_bin()
        .map(|docker_bin| DockerRuntime::with_docker_bin(config, docker_bin.to_path_buf()))
        .unwrap_or_else(|| DockerRuntime::new(config))
}

impl RuntimeProbe for DockerRuntime {
    fn observe(&self, slot: &SlotConfig) -> RuntimeObservation {
        if self.slot_mode == "fake" {
            return RuntimeObservation {
                docker_status: DockerStatus::Skipped,
                cdp_reachable: None,
                provider_readiness: ProviderReadiness::NotChecked,
            };
        }

        let docker_status = inspect_docker_status(&self.docker_bin, &slot.container);
        if docker_status == DockerStatus::Running && self.status_provider_check {
            return self.observe_provider_status(slot, docker_status);
        }

        RuntimeObservation {
            docker_status,
            cdp_reachable: None,
            provider_readiness: ProviderReadiness::NotChecked,
        }
    }
}

impl DockerRuntime {
    fn observe_provider_status(
        &self,
        slot: &SlotConfig,
        docker_status: DockerStatus,
    ) -> RuntimeObservation {
        let command = match provider_status_command(&self.docker_bin, slot, &self.state_root) {
            Ok(command) => command,
            Err(_) => return provider_unreachable(docker_status),
        };
        let output = match run_with_timeout(command, self.provider_status_timeout) {
            Ok(output) => output,
            Err(_) => return provider_unreachable(docker_status),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: Value = match serde_json::from_str(stdout.trim()) {
            Ok(value) => value,
            Err(_) => return provider_schema_drift(docker_status, output.status.success()),
        };
        let summary = match validate_provider_envelope(&value) {
            Ok(summary) => summary,
            Err(_) => return provider_schema_drift(docker_status, output.status.success()),
        };
        let provider_readiness = match provider_readiness(&summary.status) {
            Some(readiness) => readiness,
            None => return provider_schema_drift(docker_status, output.status.success()),
        };
        RuntimeObservation {
            docker_status,
            cdp_reachable: Some(!matches!(
                provider_readiness,
                ProviderReadiness::Unreachable
            )),
            provider_readiness,
        }
    }
}

fn inspect_docker_status(docker_bin: &Path, container: &str) -> DockerStatus {
    let output = Command::new(docker_bin)
        .args(["inspect", "-f", "{{.State.Status}}", container])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            match docker_status_text(&output.stdout).as_str() {
                "running" => DockerStatus::Running,
                "exited" | "dead" | "created" | "paused" | "restarting" | "removing" => {
                    DockerStatus::Exited
                }
                "" => DockerStatus::Unknown,
                _ => DockerStatus::Unknown,
            }
        }
        Ok(_) => DockerStatus::Missing,
        Err(_) => DockerStatus::Unknown,
    }
}

fn docker_status_text(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout).trim().to_ascii_lowercase()
}

fn provider_status_command(
    docker_bin: &Path,
    slot: &SlotConfig,
    state_root: &std::path::Path,
) -> Result<Command, io::Error> {
    let artifact_host_dir = state_root
        .join("slots")
        .join(&slot.slot_id.0)
        .join("artifacts")
        .join("rust-status");
    crate::provider_runner::create_private_directory(
        state_root,
        &artifact_host_dir.join("downloads"),
    )?;
    let uid = current_id("-u").unwrap_or_else(|| "1000".to_string());
    let gid = current_id("-g").unwrap_or_else(|| "1000".to_string());

    let mut command = Command::new(docker_bin);
    command
        .arg("exec")
        .arg("-i")
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .arg("--env")
        .arg(format!("BROWSER_AGENT_HOME=/state/{}", slot.slot_id.0))
        .arg("--env")
        .arg(format!("CDP_PORT={}", slot.cdp_port))
        .arg("--env")
        .arg("GPT_WEBAI_ARTIFACTS_DIR=/broker-artifacts/rust-status")
        .arg("--env")
        .arg(format!(
            "GPT_WEBAI_ARTIFACTS_HOST_DIR={}",
            artifact_host_dir.display()
        ))
        .arg(&slot.container)
        .arg("gpt-webai-provider")
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn provider_readiness(status: &str) -> Option<ProviderReadiness> {
    let health = if status == "provider.schema_drift" {
        crate::contracts::health::HealthStatus::SchemaDrift
    } else {
        crate::contracts::health::HealthStatus::parse(status)?
    };
    Some(ProviderReadiness::from_health(health))
}

fn provider_unreachable(docker_status: DockerStatus) -> RuntimeObservation {
    RuntimeObservation {
        docker_status,
        cdp_reachable: Some(false),
        provider_readiness: ProviderReadiness::Unreachable,
    }
}

fn provider_schema_drift(docker_status: DockerStatus, cdp_reachable: bool) -> RuntimeObservation {
    RuntimeObservation {
        docker_status,
        cdp_reachable: Some(cdp_reachable),
        provider_readiness: ProviderReadiness::SchemaDrift,
    }
}

fn current_id(flag: &str) -> Option<String> {
    let output = Command::new("id").arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return child.wait_with_output();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
