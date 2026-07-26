use std::path::{Component, Path};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_TIMESTAMP_MS: u64 = 9_007_199_254_740_991;
pub const MAX_DURATION_MS: u64 = 12_000_000;
pub const MAX_BYTE_COUNT: u64 = 10_737_418_240;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IdError {
    #[error("invalid {kind}: {value}")]
    Invalid { kind: &'static str, value: String },
}

pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub fn h256(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

pub fn derive_browser_context_id(
    browser_guid: &str,
    cdp_browser_context_id: &str,
) -> Result<String, IdError> {
    validate_browser_guid(browser_guid)?;
    validate_raw_string("cdpBrowserContextId", cdp_browser_context_id, true)?;
    Ok(derived_id(
        "ctx",
        json!(["pr72.ctx.r13.v1", browser_guid, cdp_browser_context_id]),
    ))
}

pub fn derive_target_id(browser_guid: &str, cdp_target_id: &str) -> Result<String, IdError> {
    validate_browser_guid(browser_guid)?;
    validate_raw_string("cdpTargetId", cdp_target_id, false)?;
    Ok(derived_id(
        "target",
        json!(["pr72.target.r13.v1", browser_guid, cdp_target_id]),
    ))
}

pub fn derive_page_incarnation_id(
    browser_guid: &str,
    cdp_target_id: &str,
    main_frame_id: &str,
    loader_id: &str,
) -> Result<String, IdError> {
    validate_browser_guid(browser_guid)?;
    validate_raw_string("cdpTargetId", cdp_target_id, false)?;
    validate_raw_string("mainFrameId", main_frame_id, false)?;
    validate_raw_string("loaderId", loader_id, false)?;
    Ok(derived_id(
        "page",
        json!([
            "pr72.page.r13.v1",
            browser_guid,
            cdp_target_id,
            main_frame_id,
            loader_id
        ]),
    ))
}

pub fn derive_page_binding_id(
    page_incarnation_id: &str,
    root_binding_hash: &str,
) -> Result<String, IdError> {
    validate_page_incarnation_id(page_incarnation_id)?;
    validate_h256(root_binding_hash)?;
    Ok(derived_id(
        "binding",
        json!([
            "pr72.page-binding.r13.v1",
            page_incarnation_id,
            root_binding_hash
        ]),
    ))
}

pub fn derive_session_binding_id(
    session_id: &str,
    slot_id: &str,
    cohort: &str,
) -> Result<String, IdError> {
    validate_session_id(session_id)?;
    validate_slot_id(slot_id)?;
    validate_cohort(cohort)?;
    Ok(derived_id(
        "binding",
        json!(["pr72.session-binding.r13.v1", session_id, slot_id, cohort]),
    ))
}

pub fn derive_turn_id(
    session_id: &str,
    author_role: &str,
    data_message_id: &str,
) -> Result<String, IdError> {
    validate_session_id(session_id)?;
    if !matches!(author_role, "user" | "assistant") {
        return Err(invalid("authorRole", author_role));
    }
    validate_raw_string("dataMessageId", data_message_id, false)?;
    Ok(derived_id(
        "turn",
        json!(["pr72.turn.r13.v1", session_id, author_role, data_message_id]),
    ))
}

pub fn chip_stem_hash(normalized_stem: &str) -> Result<String, IdError> {
    validate_raw_string("normalizedStem", normalized_stem, true)?;
    Ok(sha256_hex(normalized_stem.as_bytes()))
}

pub fn derive_chip_stable_key(
    page_incarnation_id: &str,
    normalized_stem: &str,
    duplicate_ordinal: u8,
) -> Result<String, IdError> {
    validate_page_incarnation_id(page_incarnation_id)?;
    if duplicate_ordinal > 63 {
        return Err(invalid("dupOrdinal", &duplicate_ordinal.to_string()));
    }
    Ok(format!(
        "sha256:{}",
        canonical_array_sha256(json!([
            "pr72.chip.r13.v1",
            page_incarnation_id,
            chip_stem_hash(normalized_stem)?,
            duplicate_ordinal
        ]))
    ))
}

pub fn derive_download_event_id(
    page_incarnation_id: &str,
    cdp_download_guid: &str,
    suggested_filename: &str,
) -> Result<String, IdError> {
    validate_page_incarnation_id(page_incarnation_id)?;
    validate_raw_string("cdpDownloadGuid", cdp_download_guid, false)?;
    validate_raw_string("suggestedFilename", suggested_filename, true)?;
    Ok(derived_id(
        "download",
        json!([
            "pr72.download-event.r13.v1",
            page_incarnation_id,
            cdp_download_guid,
            suggested_filename
        ]),
    ))
}

pub fn derive_artifact_id(
    artifact_claim_id: &str,
    control_id: &str,
    download_event_id: &str,
) -> Result<String, IdError> {
    validate_artifact_claim_id(artifact_claim_id)?;
    validate_control_id(control_id)?;
    validate_download_event_id(download_event_id)?;
    Ok(derived_id(
        "artifact",
        json!([
            "pr72.artifact.r13.v1",
            artifact_claim_id,
            control_id,
            download_event_id
        ]),
    ))
}

pub fn artifact_host_saved_rel_path(
    request_key: &str,
    artifact_claim_id: &str,
    artifact_id: &str,
) -> Result<String, IdError> {
    validate_request_key(request_key)?;
    validate_artifact_claim_id(artifact_claim_id)?;
    validate_artifact_id(artifact_id)?;
    let value = format!("artifacts/{request_key}/{artifact_claim_id}/{artifact_id}.download");
    validate_safe_rel_path(&value)?;
    Ok(value)
}

pub fn derive_writer_id(
    host_id: &str,
    process_id: u32,
    process_start_ms: u64,
) -> Result<String, IdError> {
    validate_host_id(host_id)?;
    if process_id == 0 {
        return Err(invalid("processId", &process_id.to_string()));
    }
    validate_timestamp_ms(process_start_ms)?;
    Ok(derived_id(
        "writer",
        json!(["pr72.writer.r13.v1", host_id, process_id, process_start_ms]),
    ))
}

pub fn derive_runtime_incarnation_id(
    slot_id: &str,
    incarnation_nonce: &str,
) -> Result<String, IdError> {
    validate_slot_id(slot_id)?;
    validate_lower_hex("incarnationNonce", incarnation_nonce, 32)?;
    Ok(derived_id(
        "runtime",
        json!([
            "pr72.runtime-incarnation.r13.v1",
            slot_id,
            incarnation_nonce
        ]),
    ))
}

fn derived_id(prefix: &str, preimage: Value) -> String {
    format!("{prefix}_{}", canonical_array_sha256(preimage))
}

fn canonical_array_sha256(preimage: Value) -> String {
    let mut bytes = serde_json::to_vec(&preimage).expect("derived identifier preimage serializes");
    bytes.push(b'\n');
    sha256_hex(bytes)
}

fn validate_browser_guid(value: &str) -> Result<(), IdError> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        });
    valid
        .then_some(())
        .ok_or_else(|| invalid("browserGuid", value))
}

fn validate_raw_string(kind: &'static str, value: &str, empty_legal: bool) -> Result<(), IdError> {
    (empty_legal || !value.is_empty())
        .then_some(())
        .filter(|_| !value.contains('\0'))
        .ok_or_else(|| invalid(kind, value))
}

fn validate_lower_hex(kind: &'static str, value: &str, digits: usize) -> Result<(), IdError> {
    (value.len() == digits
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(())
    .ok_or_else(|| invalid(kind, value))
}

pub fn validate_request_id(value: &str) -> Result<(), IdError> {
    validate_general_id("requestId", value)
}

pub fn validate_host_id(value: &str) -> Result<(), IdError> {
    value
        .strip_prefix("host_")
        .ok_or_else(|| invalid("hostId", value))
        .and_then(|hex| validate_lower_hex("hostId", hex, 32))
}

pub fn validate_run_id(value: &str) -> Result<(), IdError> {
    validate_general_id("runId", value)
}

pub fn validate_operation_id(value: &str) -> Result<(), IdError> {
    validate_general_id("operationId", value)
}

pub fn validate_session_id(value: &str) -> Result<(), IdError> {
    let valid = (1..=128).contains(&value.len())
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'-'))
        });
    valid
        .then_some(())
        .ok_or_else(|| invalid("sessionId", value))
}

pub fn validate_slot_id(value: &str) -> Result<(), IdError> {
    let valid = value
        .strip_prefix("slot-")
        .and_then(|suffix| suffix.parse::<u8>().ok())
        .is_some_and(|number| (1..=10).contains(&number) && value.len() == 7);
    valid.then_some(()).ok_or_else(|| invalid("slotId", value))
}

pub fn validate_prefixed_hex(
    kind: &'static str,
    value: &str,
    prefix: &str,
    digits: usize,
) -> Result<(), IdError> {
    let valid = value.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == digits
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    valid.then_some(()).ok_or_else(|| invalid(kind, value))
}

pub fn validate_h256(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("H256", value, "sha256:", 64)
}

pub fn validate_event_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("eventId", value, "evt_", 64)
}

pub fn validate_claim_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("claimId", value, "claim_", 64)
}

pub fn validate_lease_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("leaseId", value, "lease_", 64)
}

pub fn validate_owner_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("ownerId", value, "owner_", 64)
}

pub fn validate_release_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("releaseId", value, "release_", 64)
}

pub fn validate_receipt_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("receiptId", value, "receipt_", 64)
}

pub fn validate_artifact_claim_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("artifactClaimId", value, "artifact_claim_", 64)
}

pub fn validate_artifact_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("artifactId", value, "artifact_", 64)
}

pub fn validate_binding_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("bindingId", value, "binding_", 64)
}

pub fn validate_root_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("rootId", value, "root_", 64)
}

pub fn validate_control_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("controlId", value, "control_", 64)
}

pub fn validate_turn_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("turnId", value, "turn_", 64)
}

pub fn validate_runtime_incarnation_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("runtimeIncarnationId", value, "runtime_", 64)
}

pub fn validate_browser_context_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("browserContextId", value, "ctx_", 64)
}

pub fn validate_target_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("targetId", value, "target_", 64)
}

pub fn validate_page_incarnation_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("pageIncarnationId", value, "page_", 64)
}

pub fn validate_download_event_id(value: &str) -> Result<(), IdError> {
    validate_prefixed_hex("downloadEventId", value, "download_", 64)
}

pub fn validate_cohort(value: &str) -> Result<(), IdError> {
    matches!(value, "cohort-a" | "cohort-b" | "cohort-c")
        .then_some(())
        .ok_or_else(|| invalid("cohort", value))
}

pub fn validate_timestamp_ms(value: u64) -> Result<(), IdError> {
    (1..=MAX_TIMESTAMP_MS)
        .contains(&value)
        .then_some(())
        .ok_or_else(|| invalid("TimestampMs", &value.to_string()))
}

pub fn validate_duration_ms(value: u64) -> Result<(), IdError> {
    (value <= MAX_DURATION_MS)
        .then_some(())
        .ok_or_else(|| invalid("DurationMs", &value.to_string()))
}

pub fn validate_byte_count(value: u64) -> Result<(), IdError> {
    (value <= MAX_BYTE_COUNT)
        .then_some(())
        .ok_or_else(|| invalid("ByteCount", &value.to_string()))
}

pub fn validate_generation(value: u16) -> Result<(), IdError> {
    (value > 0)
        .then_some(())
        .ok_or_else(|| invalid("generation", &value.to_string()))
}

pub fn validate_request_key(value: &str) -> Result<(), IdError> {
    let valid = value
        .strip_prefix("r-")
        .is_some_and(|id| validate_request_id(id).is_ok())
        || value
            .strip_prefix("s-")
            .is_some_and(|id| validate_session_id(id).is_ok())
        || value
            .strip_prefix("d-")
            .is_some_and(|id| validate_operation_id(id).is_ok());
    valid
        .then_some(())
        .ok_or_else(|| invalid("RequestKey", value))
}

pub fn validate_conversation_url(value: &str, session_id: &str) -> Result<(), IdError> {
    validate_session_id(session_id)?;
    let expected = format!("https://chatgpt.com/c/{session_id}");
    (value == expected)
        .then_some(())
        .ok_or_else(|| invalid("conversationUrl", value))
}

pub fn validate_safe_rel_path(value: &str) -> Result<(), IdError> {
    let raw_components_valid = value
        .split('/')
        .all(|component| !component.is_empty() && !matches!(component, "." | ".."));
    let valid = (1..=240).contains(&value.len())
        && !value.contains(['\0', '\\'])
        && !value.chars().any(char::is_control)
        && !Path::new(value).is_absolute()
        && raw_components_valid
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(part) if !part.is_empty()));
    valid
        .then_some(())
        .ok_or_else(|| invalid("SafeRelPath", value))
}

pub fn validate_non_empty_text(value: &str) -> Result<(), IdError> {
    validate_text("NonEmptyText", value, 4_096)
}

pub fn validate_answer_text(value: &str) -> Result<(), IdError> {
    validate_text("AnswerText", value, 65_536)
}

fn validate_text(kind: &'static str, value: &str, max_bytes: usize) -> Result<(), IdError> {
    (!value.is_empty() && value.len() <= max_bytes && !value.contains('\0'))
        .then_some(())
        .ok_or_else(|| invalid(kind, value))
}

fn validate_general_id(kind: &'static str, value: &str) -> Result<(), IdError> {
    let valid = (1..=128).contains(&value.len())
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b':' | b'-'))
        });
    valid.then_some(()).ok_or_else(|| invalid(kind, value))
}

fn invalid(kind: &'static str, value: &str) -> IdError {
    IdError::Invalid {
        kind,
        value: value.to_string(),
    }
}
