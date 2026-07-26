use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use thiserror::Error;

use crate::provider_client::{ProviderInvocationError, PROVIDER_SCHEMA};
use crate::provider_runner::{ProviderCommand, ProviderPathMode};
use crate::wait_gate::{confirm_pre_poll_wait_gate, PrePollWaitGateEvidence, WaitGateError};

use super::input::RequestRunInput;
use super::provider::provider_capture_session;

const PRE_POLL_WAIT_GATE_LABEL: &str = "pre-poll-wait-gate";

#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestWaitGateInput<'a> {
    pub request: &'a RequestRunInput,
    pub command: &'a ProviderCommand,
    pub session_id: &'a str,
    pub conversation_url: &'a str,
    pub real_turn_evidence: bool,
}

#[derive(Debug, Error)]
pub enum RequestWaitGateError {
    #[error("provider capture failed: {0}")]
    Capture(String),
    #[error("wait gate failed: {0}")]
    Gate(#[from] WaitGateError),
}

#[derive(Debug, Error)]
enum SavedCaptureRecoveryError {
    #[error("artifact host directory unavailable")]
    ArtifactHostDirUnavailable,
    #[error("saved diagnostic file missing: {0}")]
    MissingFile(String),
    #[error("saved diagnostic JSON invalid at {path}: {source}")]
    Json {
        path: String,
        source: serde_json::Error,
    },
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("saved diagnostic URL missing or not a conversation URL")]
    ConversationUrlMissing,
    #[error("saved diagnostic URL mismatch: expected {expected}, got {actual}")]
    ConversationUrlMismatch { expected: String, actual: String },
}

pub(crate) fn run_pre_poll_wait_gate(
    input: RequestWaitGateInput<'_>,
) -> Result<(), RequestWaitGateError> {
    match provider_capture_session(input.request, input.command, input.session_id) {
        Ok(capture) => {
            confirm_pre_poll_wait_gate(PrePollWaitGateEvidence {
                capture_value: &capture.value,
                session_id: input.session_id,
                conversation_url: input.conversation_url,
                real_turn_evidence: input.real_turn_evidence,
            })?;
            Ok(())
        }
        Err(error @ ProviderInvocationError::Timeout(_)) => {
            recover_saved_capture_after_timeout(input, error.to_string())
        }
        Err(error) => Err(RequestWaitGateError::Capture(error.to_string())),
    }
}

fn recover_saved_capture_after_timeout(
    input: RequestWaitGateInput<'_>,
    capture_error: String,
) -> Result<(), RequestWaitGateError> {
    let recovered = saved_pre_poll_capture(input.command, input.session_id, input.conversation_url)
        .map_err(|error| {
            RequestWaitGateError::Capture(format!(
                "{capture_error}; saved capture recovery unavailable: {error}"
            ))
        })?;
    confirm_pre_poll_wait_gate(PrePollWaitGateEvidence {
        capture_value: &recovered,
        session_id: input.session_id,
        conversation_url: input.conversation_url,
        real_turn_evidence: input.real_turn_evidence,
    })
    .map_err(|error| {
        RequestWaitGateError::Capture(format!(
            "{capture_error}; saved capture recovery failed: {error}"
        ))
    })
}

fn saved_pre_poll_capture(
    command: &ProviderCommand,
    session_id: &str,
    conversation_url: &str,
) -> Result<Value, SavedCaptureRecoveryError> {
    let artifact_dir =
        artifact_host_dir(command).ok_or(SavedCaptureRecoveryError::ArtifactHostDirUnavailable)?;
    let diagnostics_dir = artifact_dir.join("diagnostics");
    let screenshot_path = diagnostics_dir.join(format!("{PRE_POLL_WAIT_GATE_LABEL}.png"));
    let crop_path = diagnostics_dir.join(format!(
        "{PRE_POLL_WAIT_GATE_LABEL}.right-edge-scrollbar.png"
    ));
    let dom_path = diagnostics_dir.join(format!("{PRE_POLL_WAIT_GATE_LABEL}.dom.json"));
    let proof_path = diagnostics_dir.join(format!("{PRE_POLL_WAIT_GATE_LABEL}.scroll-proof.json"));

    require_file(&screenshot_path)?;
    require_file(&crop_path)?;
    require_file(&dom_path)?;
    require_file(&proof_path)?;

    let dom = read_json(&dom_path)?;
    let proof = read_json(&proof_path)?;
    let saved_url = dom
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://chatgpt.com/c/"))
        .ok_or(SavedCaptureRecoveryError::ConversationUrlMissing)?;
    if saved_url != conversation_url {
        return Err(SavedCaptureRecoveryError::ConversationUrlMismatch {
            expected: conversation_url.to_string(),
            actual: saved_url.to_string(),
        });
    }

    let mut diagnostics = serde_json::Map::new();
    diagnostics.insert("label".to_string(), json!(PRE_POLL_WAIT_GATE_LABEL));
    diagnostics.insert("screenshot".to_string(), json!("saved"));
    diagnostics.insert("dom".to_string(), json!("saved"));
    diagnostics.insert(
        "screenshotPath".to_string(),
        json!(path_text(&screenshot_path)),
    );
    diagnostics.insert("domPath".to_string(), json!(path_text(&dom_path)));
    diagnostics.insert(
        "rightEdgeScrollbarCropPath".to_string(),
        json!(path_text(&crop_path)),
    );
    diagnostics.insert(
        "scrollBottomProofPath".to_string(),
        json!(path_text(&proof_path)),
    );
    diagnostics.insert("url".to_string(), json!(saved_url));
    copy_field(&dom, &mut diagnostics, "title");
    copy_field(&dom, &mut diagnostics, "readinessSignals");
    copy_field(&dom, &mut diagnostics, "selectorInventory");
    copy_field(&dom, &mut diagnostics, "dialogs");
    copy_field(&dom, &mut diagnostics, "providerLimitSurfaces");
    copy_field(&dom, &mut diagnostics, "fullViewportScreenshot");
    copy_field(&dom, &mut diagnostics, "rightEdgeScrollbarCrop");
    copy_field(&dom, &mut diagnostics, "bottomScroll");
    diagnostics.insert("scrollBottomProof".to_string(), proof);

    Ok(json!({
        "schema": PROVIDER_SCHEMA,
        "ok": true,
        "vendor": "chatgpt",
        "status": "captured",
        "sessionId": session_id,
        "conversationUrl": conversation_url,
        "diagnostics": Value::Object(diagnostics),
    }))
}

fn artifact_host_dir(command: &ProviderCommand) -> Option<PathBuf> {
    match &command.path_mode {
        ProviderPathMode::DockerSlot(paths) => Some(paths.artifact_host_dir.clone()),
        ProviderPathMode::Host => env_path(command, "GPT_WEBAI_ARTIFACTS_HOST_DIR")
            .or_else(|| env_path(command, "GPT_WEBAI_ARTIFACTS_DIR"))
            .or_else(|| std::env::var_os("GPT_WEBAI_ARTIFACTS_HOST_DIR").map(PathBuf::from))
            .or_else(|| std::env::var_os("GPT_WEBAI_ARTIFACTS_DIR").map(PathBuf::from)),
    }
}

fn env_path(command: &ProviderCommand, name: &str) -> Option<PathBuf> {
    command.env.iter().find_map(|(key, value)| {
        (key == name && !value.is_empty()).then(|| PathBuf::from(value.as_str()))
    })
}

fn require_file(path: &Path) -> Result<(), SavedCaptureRecoveryError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(SavedCaptureRecoveryError::MissingFile(path_text(path)))
    }
}

fn read_json(path: &Path) -> Result<Value, SavedCaptureRecoveryError> {
    let text = fs::read_to_string(path).map_err(|source| SavedCaptureRecoveryError::Io {
        path: path_text(path),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| SavedCaptureRecoveryError::Json {
        path: path_text(path),
        source,
    })
}

fn copy_field(source: &Value, target: &mut serde_json::Map<String, Value>, name: &str) {
    if let Some(value) = source.get(name) {
        target.insert(name.to_string(), value.clone());
    }
}

fn path_text(path: &Path) -> String {
    path.display().to_string()
}
