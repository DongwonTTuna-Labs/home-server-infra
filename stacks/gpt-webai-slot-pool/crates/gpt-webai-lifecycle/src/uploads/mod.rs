pub mod recovery;
pub mod staging;

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::contracts::browser::EvidenceRef;
use crate::contracts::ids::{
    validate_byte_count, validate_h256, validate_non_empty_text, validate_request_id,
    validate_run_id, validate_safe_rel_path,
};
use crate::journal::canonical::canonical_bytes;

pub const MAX_ATTACHMENTS: usize = 64;
pub const UPLOAD_PROOF_MAX_AGE_MS: u64 = 30_000;
pub const ATTACHMENT_CONTAINER_ROOT: &str = "/broker-attachments/";
pub const PROMPT_CONTAINER_ROOT: &str = "/broker-prompts/";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AttachmentRecord {
    pub ordinal: u8,
    pub source_sha256: String,
    pub size_bytes: u64,
    pub staged_rel_path: String,
    pub container_rel_path: String,
    pub media_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AttachmentSet {
    pub count: u8,
    pub records: Vec<AttachmentRecord>,
    pub set_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PromptInput {
    pub container_rel_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChipProof {
    pub chip_stable_key: String,
    pub label_hash: String,
    pub visible_size_bytes: Option<u64>,
    pub digest: Option<String>,
    pub bounding_box_hash: String,
    pub complete: bool,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UploadProof {
    pub upload_attempt_id: String,
    pub retry_index: u8,
    pub expected_set_sha256: String,
    pub visible_current_chips: Vec<ChipProof>,
    pub stale_chips: Vec<ChipProof>,
    pub all_expected_complete: bool,
    pub captured_at_ms: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum UploadContractError {
    #[error("invalid upload contract field: {0}")]
    Invalid(&'static str),
    #[error("upload set hash could not be encoded")]
    Encoding,
}

impl AttachmentSet {
    pub fn from_records(records: Vec<AttachmentRecord>) -> Result<Self, UploadContractError> {
        if records.len() > MAX_ATTACHMENTS {
            return Err(UploadContractError::Invalid("records"));
        }
        let set_sha256 = hash_records(&records)?;
        Ok(Self {
            count: records.len() as u8,
            records,
            set_sha256,
        })
    }

    pub fn validate_for(&self, request_id: &str, run_id: &str) -> Result<(), UploadContractError> {
        valid(validate_request_id(request_id), "requestId")?;
        valid(validate_run_id(run_id), "runId")?;
        if self.records.len() > MAX_ATTACHMENTS
            || usize::from(self.count) != self.records.len()
            || self.set_sha256 != hash_records(&self.records)?
        {
            return Err(UploadContractError::Invalid("attachmentSet"));
        }
        for (ordinal, record) in self.records.iter().enumerate() {
            record.validate_for(request_id, run_id, ordinal as u8)?;
        }
        Ok(())
    }
}

impl AttachmentRecord {
    fn validate_for(
        &self,
        request_id: &str,
        run_id: &str,
        expected_ordinal: u8,
    ) -> Result<(), UploadContractError> {
        if self.ordinal != expected_ordinal || self.ordinal >= MAX_ATTACHMENTS as u8 {
            return Err(UploadContractError::Invalid("ordinal"));
        }
        valid(validate_h256(&self.source_sha256), "sourceSha256")?;
        valid(validate_byte_count(self.size_bytes), "sizeBytes")?;
        valid(
            validate_safe_rel_path(&self.staged_rel_path),
            "stagedRelPath",
        )?;
        valid(
            validate_safe_rel_path(&self.container_rel_path),
            "containerRelPath",
        )?;
        valid(validate_non_empty_text(&self.media_type), "mediaType")?;
        let prefix = format!("requests/{request_id}/attachments/{run_id}/");
        let container_prefix = format!("{run_id}/");
        let staged_name = self
            .staged_rel_path
            .strip_prefix(&prefix)
            .ok_or(UploadContractError::Invalid("stagedRelPath root"))?;
        let container_name = self
            .container_rel_path
            .strip_prefix(&container_prefix)
            .ok_or(UploadContractError::Invalid("containerRelPath root"))?;
        if staged_name != container_name || !valid_staging_name(staged_name, self) {
            return Err(UploadContractError::Invalid("staging filename"));
        }
        Ok(())
    }
}

impl PromptInput {
    pub fn validate_for(
        &self,
        run_id: &str,
        expected_sha256: &str,
    ) -> Result<(), UploadContractError> {
        valid(validate_run_id(run_id), "runId")?;
        valid(
            validate_safe_rel_path(&self.container_rel_path),
            "containerRelPath",
        )?;
        valid(validate_h256(&self.sha256), "sha256")?;
        valid(validate_byte_count(self.size_bytes), "sizeBytes")?;
        if self.container_rel_path != format!("{run_id}/prompt.txt")
            || self.sha256 != expected_sha256
        {
            return Err(UploadContractError::Invalid("prompt binding"));
        }
        Ok(())
    }
}

fn hash_records(records: &[AttachmentRecord]) -> Result<String, UploadContractError> {
    canonical_bytes(&records)
        .map(crate::contracts::ids::h256)
        .map_err(|_| UploadContractError::Encoding)
}

fn valid_staging_name(name: &str, record: &AttachmentRecord) -> bool {
    let Some(hex) = record.source_sha256.strip_prefix("sha256:") else {
        return false;
    };
    let required = format!("{:03}-{}", usize::from(record.ordinal) + 1, &hex[..16]);
    let suffix = match name.strip_prefix(&required) {
        Some("") => return true,
        Some(value) => value,
        None => return false,
    };
    suffix.strip_prefix('.').is_some_and(|extension| {
        (1..=8).contains(&extension.len())
            && extension
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    })
}

fn valid<T, E>(result: Result<T, E>, field: &'static str) -> Result<(), UploadContractError> {
    result
        .map(|_| ())
        .map_err(|_| UploadContractError::Invalid(field))
}

pub(crate) fn digest_reader(
    reader: &mut File,
) -> Result<(sha2::digest::Output<Sha256>, u64), io::Error> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((hasher.finalize(), size))
}

pub(crate) fn copy_and_digest(
    reader: &mut File,
    writer: &mut File,
) -> Result<(sha2::digest::Output<Sha256>, u64), io::Error> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((hasher.finalize(), size))
}

pub(crate) fn stable_metadata(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    first.dev() == second.dev()
        && first.ino() == second.ino()
        && first.len() == second.len()
        && first.mtime() == second.mtime()
        && first.mtime_nsec() == second.mtime_nsec()
        && first.ctime() == second.ctime()
        && first.ctime_nsec() == second.ctime_nsec()
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(
        path.parent()
            .ok_or_else(|| io::Error::other("missing parent"))?,
    )?
    .sync_all()
}
