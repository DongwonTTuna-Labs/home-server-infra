use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::contracts::browser::SessionEcho;
use crate::contracts::ids::{
    derive_artifact_id, validate_artifact_claim_id, validate_artifact_id,
    validate_browser_context_id, validate_byte_count, validate_conversation_url,
    validate_download_event_id, validate_h256, validate_non_empty_text,
    validate_page_incarnation_id, validate_request_key, validate_safe_rel_path,
    validate_session_id, validate_slot_id, validate_target_id, validate_timestamp_ms,
    validate_turn_id,
};

use super::{valid, ArtifactClaimError, ArtifactControl};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PlaywrightDownloadReceipt {
    pub artifact_claim_id: String,
    pub artifact_id: String,
    pub browser_context_id: String,
    pub clicked_at_ms: u64,
    pub control: ArtifactControl,
    pub conversation_url: String,
    pub download_event_id: String,
    pub host_saved_rel_path: String,
    pub listener_armed_at_ms: u64,
    pub media_type: String,
    pub page_incarnation_id: String,
    pub received_at_ms: u64,
    pub session_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub slot_id: String,
    pub target_id: String,
    pub terminal_assistant_turn_id: String,
}

impl PlaywrightDownloadReceipt {
    pub fn validate_shape(&self) -> Result<(), ArtifactClaimError> {
        for (result, field) in [
            (
                validate_artifact_claim_id(&self.artifact_claim_id),
                "artifactClaimId",
            ),
            (validate_artifact_id(&self.artifact_id), "artifactId"),
            (
                validate_browser_context_id(&self.browser_context_id),
                "browserContextId",
            ),
            (
                validate_download_event_id(&self.download_event_id),
                "downloadEventId",
            ),
            (
                validate_page_incarnation_id(&self.page_incarnation_id),
                "pageIncarnationId",
            ),
            (validate_session_id(&self.session_id), "sessionId"),
            (validate_h256(&self.sha256), "sha256"),
            (validate_slot_id(&self.slot_id), "slotId"),
            (validate_target_id(&self.target_id), "targetId"),
            (
                validate_turn_id(&self.terminal_assistant_turn_id),
                "terminalAssistantTurnId",
            ),
        ] {
            valid(result, field)?;
        }
        valid(
            validate_safe_rel_path(&self.host_saved_rel_path),
            "hostSavedRelPath",
        )?;
        valid(validate_non_empty_text(&self.media_type), "mediaType")?;
        valid(validate_byte_count(self.size_bytes), "sizeBytes")?;
        valid(
            validate_timestamp_ms(self.listener_armed_at_ms),
            "listenerArmedAtMs",
        )?;
        valid(validate_timestamp_ms(self.clicked_at_ms), "clickedAtMs")?;
        valid(validate_timestamp_ms(self.received_at_ms), "receivedAtMs")?;
        valid(
            validate_conversation_url(&self.conversation_url, &self.session_id),
            "conversationUrl",
        )?;
        self.control
            .validate_for_turn(&self.terminal_assistant_turn_id)?;
        let expected_artifact_id = derive_artifact_id(
            &self.artifact_claim_id,
            &self.control.control_id,
            &self.download_event_id,
        )
        .map_err(|_| ArtifactClaimError::Invalid("artifactId preimage"))?;
        validate_host_saved_rel_path(
            &self.host_saved_rel_path,
            &self.artifact_claim_id,
            &expected_artifact_id,
        )?;
        if self.size_bytes == 0
            || self.artifact_id != expected_artifact_id
            || !(self.listener_armed_at_ms < self.clicked_at_ms
                && self.clicked_at_ms <= self.received_at_ms)
            || self.media_type != mime_from_path(&self.host_saved_rel_path)
        {
            return Err(ArtifactClaimError::Invalid("download receipt shape"));
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        expected: &SessionEcho,
        claim_id: &str,
        control: &ArtifactControl,
        terminal_turn_id: &str,
    ) -> Result<(), ArtifactClaimError> {
        self.validate_shape()?;
        for (result, field) in [
            (
                validate_artifact_claim_id(&self.artifact_claim_id),
                "artifactClaimId",
            ),
            (validate_artifact_id(&self.artifact_id), "artifactId"),
            (
                validate_browser_context_id(&self.browser_context_id),
                "browserContextId",
            ),
            (
                validate_download_event_id(&self.download_event_id),
                "downloadEventId",
            ),
            (
                validate_page_incarnation_id(&self.page_incarnation_id),
                "pageIncarnationId",
            ),
            (validate_session_id(&self.session_id), "sessionId"),
            (validate_h256(&self.sha256), "sha256"),
            (validate_slot_id(&self.slot_id), "slotId"),
            (validate_target_id(&self.target_id), "targetId"),
            (
                validate_turn_id(&self.terminal_assistant_turn_id),
                "terminalAssistantTurnId",
            ),
        ] {
            valid(result, field)?;
        }
        let page = &expected.page_binding;
        if self.artifact_claim_id != claim_id
            || &self.control != control
            || self.terminal_assistant_turn_id != terminal_turn_id
            || self.session_id != expected.session_id
            || self.conversation_url != expected.conversation_url
            || self.slot_id != page.slot_id
            || self.browser_context_id != page.browser_context_id
            || self.target_id != page.target_id
            || self.page_incarnation_id != page.page_incarnation_id
            || !(self.listener_armed_at_ms < self.clicked_at_ms
                && self.clicked_at_ms <= self.received_at_ms)
            || self.media_type != mime_from_path(&self.host_saved_rel_path)
        {
            return Err(ArtifactClaimError::Invalid("download receipt binding"));
        }
        Ok(())
    }
}

pub fn reopen_and_verify(
    state_root: &Path,
    receipt: &PlaywrightDownloadReceipt,
) -> Result<(), ArtifactClaimError> {
    let path = resolve_beneath(state_root, &receipt.host_saved_rel_path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() != receipt.size_bytes {
        return Err(ArtifactClaimError::Invalid("download file metadata"));
    }
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    if size != receipt.size_bytes || format!("sha256:{:x}", hasher.finalize()) != receipt.sha256 {
        return Err(ArtifactClaimError::Invalid("download file digest"));
    }
    Ok(())
}

pub fn mime_from_path(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("gz") => "application/gzip",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn validate_host_saved_rel_path(
    value: &str,
    artifact_claim_id: &str,
    artifact_id: &str,
) -> Result<(), ArtifactClaimError> {
    let components: Vec<_> = value.split('/').collect();
    if components.len() != 4
        || components[0] != "artifacts"
        || validate_request_key(components[1]).is_err()
        || components[2] != artifact_claim_id
        || components[3] != format!("{artifact_id}.download")
    {
        return Err(ArtifactClaimError::Invalid("hostSavedRelPath canonical"));
    }
    Ok(())
}

fn resolve_beneath(root: &Path, rel: &str) -> Result<std::path::PathBuf, ArtifactClaimError> {
    valid(validate_safe_rel_path(rel), "hostSavedRelPath")?;
    let mut current = root.to_path_buf();
    let root_meta = fs::symlink_metadata(&current).map_err(io_error)?;
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(ArtifactClaimError::Invalid("stateRoot"));
    }
    for component in rel.split('/') {
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactClaimError::Invalid("symlink path"));
        }
    }
    Ok(current)
}

fn io_error(error: std::io::Error) -> ArtifactClaimError {
    ArtifactClaimError::Io(error.to_string())
}
