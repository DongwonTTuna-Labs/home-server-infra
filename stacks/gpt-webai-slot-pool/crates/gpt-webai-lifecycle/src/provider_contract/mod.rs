use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub const PROVIDER_SCHEMA: &str = "gpt-webai.provider.envelope.v2";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderEnvelopeSummary {
    pub schema: String,
    pub ok: bool,
    pub vendor: String,
    pub status: String,
    pub reason: Option<String>,
    pub session_id: Option<String>,
    pub conversation_url: Option<String>,
    pub artifacts: usize,
    pub artifact_candidates: usize,
    pub artifact_expectation: Option<String>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProviderContractError {
    #[error("provider envelope is not a JSON object")]
    NotObject,
    #[error("provider envelope missing or invalid field: {0}")]
    InvalidField(&'static str),
    #[error("provider envelope has unknown status: {0}")]
    UnknownStatus(String),
    #[error("provider artifact object invalid at {array}[{index}]: {field}")]
    InvalidArtifactObject {
        array: &'static str,
        index: usize,
        field: &'static str,
    },
}

pub fn validate_provider_envelope(
    value: &Value,
) -> Result<ProviderEnvelopeSummary, ProviderContractError> {
    let object = value.as_object().ok_or(ProviderContractError::NotObject)?;
    let schema = string_field(object, "schema")?;
    if schema != PROVIDER_SCHEMA {
        return Err(ProviderContractError::InvalidField("schema"));
    }
    let ok = object
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or(ProviderContractError::InvalidField("ok"))?;
    let vendor = string_field(object, "vendor")?;
    if vendor != "chatgpt" {
        return Err(ProviderContractError::InvalidField("vendor"));
    }
    let status = string_field(object, "status")?;
    if !known_status(status) {
        return Err(ProviderContractError::UnknownStatus(status.to_string()));
    }
    let reason = optional_string_field(object, "reason")?;
    let session_id = optional_string_field(object, "sessionId")?;
    let conversation_url = optional_string_field(object, "conversationUrl")?;

    if matches!(
        status,
        "sent"
            | "artifact.download_timeout"
            | "artifact.controls_absent"
            | "artifact.recovery_failed"
            | "session.running_unverified"
            | "scroll.bottom_unverified"
    ) && session_id.is_none()
    {
        return Err(ProviderContractError::InvalidField("sessionId"));
    }
    let artifact_expectation = optional_artifact_expectation(object, "artifactExpectation")?;
    let artifacts = validate_artifact_array(object.get("artifacts"), "artifacts")?;
    let artifact_candidates =
        validate_artifact_array(object.get("artifactCandidates"), "artifactCandidates")?;

    Ok(ProviderEnvelopeSummary {
        schema: schema.to_string(),
        ok,
        vendor: vendor.to_string(),
        status: status.to_string(),
        reason,
        session_id,
        conversation_url,
        artifacts,
        artifact_candidates,
        artifact_expectation,
    })
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ProviderContractError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ProviderContractError::InvalidField(field))
}

fn optional_string_field(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ProviderContractError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err(ProviderContractError::InvalidField(field)),
    }
}

fn optional_artifact_expectation(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ProviderContractError> {
    let Some(value) = optional_string_field(object, field)? else {
        return Ok(None);
    };
    if matches!(value.as_str(), "none" | "optional" | "required" | "claimed") {
        Ok(Some(value))
    } else {
        Err(ProviderContractError::InvalidField(field))
    }
}

fn known_status(status: &str) -> bool {
    matches!(
        status,
        "ready"
            | "login_required"
            | "provider_limit"
            | "subscription_required"
            | "unknown"
            | "unreachable"
            | "sent"
            | "session.start_unconfirmed"
            | "session.content_unavailable"
            | "session.running_unverified"
            | "attachment_unavailable"
            | "model.selection_mismatch"
            | "running"
            | "done"
            | "artifact.download_timeout"
            | "artifact.controls_absent"
            | "artifact.recovery_failed"
            | "captured"
            | "capture_failed"
            | "scroll.bottom_unverified"
            | "resumed"
            | "show"
            | "provider.schema_drift"
    )
}

fn validate_artifact_array(
    value: Option<&Value>,
    array: &'static str,
) -> Result<usize, ProviderContractError> {
    let Some(value) = value else {
        return Ok(0);
    };
    let items = value
        .as_array()
        .ok_or(ProviderContractError::InvalidField(array))?;
    for (index, item) in items.iter().enumerate() {
        validate_artifact_object(item, array, index)?;
    }
    Ok(items.len())
}

fn validate_artifact_object(
    value: &Value,
    array: &'static str,
    index: usize,
) -> Result<(), ProviderContractError> {
    let object = value
        .as_object()
        .ok_or(ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "object",
        })?;
    let button_text = object
        .get("buttonText")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if button_text.is_empty() {
        return Err(ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "buttonText",
        });
    }
    let button_sha = object
        .get("buttonTextSha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_sha256(button_sha) {
        return Err(ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "buttonTextSha256",
        });
    }
    if object
        .get("clickedElement")
        .and_then(Value::as_object)
        .is_none()
    {
        return Err(ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "clickedElement",
        });
    }
    let artifact = object.get("artifact").and_then(Value::as_object).ok_or(
        ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "artifact",
        },
    )?;
    let status = artifact
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(status, "saved" | "failed") {
        return Err(ProviderContractError::InvalidArtifactObject {
            array,
            index,
            field: "artifact.status",
        });
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.as_bytes().iter().all(u8::is_ascii_hexdigit)
}
