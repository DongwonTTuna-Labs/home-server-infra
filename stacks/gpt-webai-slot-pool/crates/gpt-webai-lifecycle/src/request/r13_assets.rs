use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::contracts::ids::h256;
use crate::uploads::staging::{stage_attachments, stage_prompt, AttachmentSource, StagingError};
use crate::uploads::{AttachmentSet, PromptInput};

use super::input::RequestRunInput;

#[derive(Clone, Debug)]
pub struct StagedFreshAssets {
    pub attachment_set: AttachmentSet,
    pub prompt_input: PromptInput,
    pub prompt_sha256: String,
    pub prompt_size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializedFile {
    pub container_rel_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Error)]
pub enum FreshAssetError {
    #[error("R13 asset staging failed: {0}")]
    Staging(#[from] StagingError),
    #[error("R13 asset I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("R13 asset source is unsafe")]
    UnsafeSource,
    #[error("R13 materialized asset collision")]
    Collision,
}

pub fn stage_fresh_assets(input: &RequestRunInput) -> Result<StagedFreshAssets, FreshAssetError> {
    drop(crate::provider_runner::ensure_private_state_root(
        &input.config.state_root,
    )?);
    let prompt = read_safe(&input.prompt_file)?;
    let prompt_sha256 = h256(&prompt);
    let prompt_input = stage_prompt(
        &input.config.state_root,
        &input.request_id,
        &input.run_id,
        &prompt,
        &prompt_sha256,
    )?;
    let media_types = input
        .files
        .iter()
        .map(|path| media_type(path).to_string())
        .collect::<Vec<_>>();
    let sources = input
        .files
        .iter()
        .zip(&media_types)
        .map(|(path, media_type)| AttachmentSource { path, media_type })
        .collect::<Vec<_>>();
    let attachment_set = stage_attachments(
        &input.config.state_root,
        &input.request_id,
        &input.run_id,
        &sources,
    )?;
    Ok(StagedFreshAssets {
        attachment_set,
        prompt_input,
        prompt_sha256,
        prompt_size_bytes: prompt.len() as u64,
    })
}

pub fn materialize_for_slot(
    input: &RequestRunInput,
    slot_id: &str,
    assets: &StagedFreshAssets,
) -> Result<Vec<MaterializedFile>, FreshAssetError> {
    let slot_root = input.config.state_root.join("slots").join(slot_id);
    let prompt_target = slot_root
        .join("prompts")
        .join(&input.run_id)
        .join("prompt.txt");
    let prompt_source = input
        .config
        .state_root
        .join("requests")
        .join(&input.request_id)
        .join("prompt")
        .join(&input.run_id)
        .join("prompt.txt");
    copy_verified(
        &input.config.state_root,
        &prompt_source,
        &prompt_target,
        &assets.prompt_input.sha256,
        assets.prompt_input.size_bytes,
    )?;

    let mut materialized = Vec::with_capacity(assets.attachment_set.records.len());
    for record in &assets.attachment_set.records {
        let source = input.config.state_root.join(&record.staged_rel_path);
        let target = slot_root
            .join("attachments")
            .join(&record.container_rel_path);
        copy_verified(
            &input.config.state_root,
            &source,
            &target,
            &record.source_sha256,
            record.size_bytes,
        )?;
        materialized.push(MaterializedFile {
            container_rel_path: record.container_rel_path.clone(),
            sha256: record.source_sha256.clone(),
            size_bytes: record.size_bytes,
        });
    }
    Ok(materialized)
}

fn read_safe(path: &Path) -> Result<Vec<u8>, FreshAssetError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
        return Err(FreshAssetError::UnsafeSource);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn copy_verified(
    state_root: &Path,
    source: &Path,
    target: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), FreshAssetError> {
    let bytes = read_safe(source)?;
    if bytes.len() as u64 != expected_size || h256(&bytes) != expected_sha256 {
        return Err(FreshAssetError::Collision);
    }
    let parent = target.parent().ok_or(FreshAssetError::Collision)?;
    crate::provider_runner::create_private_directory(state_root, parent)?;
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(target)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            File::open(parent)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_safe(target)? != bytes {
                return Err(FreshAssetError::Collision);
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
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
