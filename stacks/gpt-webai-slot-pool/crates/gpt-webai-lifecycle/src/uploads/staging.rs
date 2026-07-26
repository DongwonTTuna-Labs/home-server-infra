use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::contracts::ids::{
    h256, validate_byte_count, validate_non_empty_text, validate_request_id, validate_run_id,
};

use super::{
    copy_and_digest, digest_reader, stable_metadata, sync_parent, AttachmentRecord, AttachmentSet,
    PromptInput, UploadContractError, MAX_ATTACHMENTS,
};

#[derive(Clone, Copy, Debug)]
pub struct AttachmentSource<'a> {
    pub path: &'a Path,
    pub media_type: &'a str,
}

#[derive(Debug, Error)]
pub enum StagingError {
    #[error("invalid staging input: {0}")]
    Invalid(&'static str),
    #[error("unsafe attachment source")]
    UnsafeSource,
    #[error("attachment changed while staging")]
    UnstableSource,
    #[error("immutable staged file collision")]
    ImmutableCollision,
    #[error("upload contract error: {0}")]
    Contract(#[from] UploadContractError),
    #[error("staging I/O failure: {0}")]
    Io(#[from] io::Error),
}

pub fn stage_attachments(
    state_root: &Path,
    request_id: &str,
    run_id: &str,
    sources: &[AttachmentSource<'_>],
) -> Result<AttachmentSet, StagingError> {
    validate_scope(state_root, request_id, run_id)?;
    if sources.len() > MAX_ATTACHMENTS {
        return Err(StagingError::Invalid("attachment count"));
    }
    let root = managed_dir(state_root, &["requests", request_id, "attachments", run_id])?;
    let mut records = Vec::with_capacity(sources.len());
    for (ordinal, source) in sources.iter().enumerate() {
        records.push(stage_one(&root, request_id, run_id, ordinal as u8, source)?);
    }
    let set = AttachmentSet::from_records(records)?;
    set.validate_for(request_id, run_id)?;
    Ok(set)
}

pub fn stage_prompt(
    state_root: &Path,
    request_id: &str,
    run_id: &str,
    prompt: &[u8],
    expected_sha256: &str,
) -> Result<PromptInput, StagingError> {
    validate_scope(state_root, request_id, run_id)?;
    validate_byte_count(prompt.len() as u64).map_err(|_| StagingError::Invalid("prompt size"))?;
    let digest = h256(prompt);
    if digest != expected_sha256 {
        return Err(StagingError::Invalid("promptSha256"));
    }
    let root = managed_dir(state_root, &["requests", request_id, "prompt", run_id])?;
    let target = root.join("prompt.txt");
    write_immutable(&target, prompt)?;
    let input = PromptInput {
        container_rel_path: format!("{run_id}/prompt.txt"),
        sha256: digest,
        size_bytes: prompt.len() as u64,
    };
    input.validate_for(run_id, expected_sha256)?;
    Ok(input)
}

fn stage_one(
    root: &Path,
    request_id: &str,
    run_id: &str,
    ordinal: u8,
    source: &AttachmentSource<'_>,
) -> Result<AttachmentRecord, StagingError> {
    validate_non_empty_text(source.media_type).map_err(|_| StagingError::Invalid("mediaType"))?;
    let mut input = open_safe_source(source.path)?;
    let before = input.metadata()?;
    let (first_hash, size) = digest_reader(&mut input)?;
    let after_hash = input.metadata()?;
    if !stable_metadata(&before, &after_hash) || size != before.len() {
        return Err(StagingError::UnstableSource);
    }
    let digest_hex = format!("{first_hash:x}");
    let filename = format!(
        "{:03}-{}{}",
        usize::from(ordinal) + 1,
        &digest_hex[..16],
        normalized_extension(source.path)
    );
    let target = root.join(&filename);
    input.seek(SeekFrom::Start(0))?;
    copy_immutable(&mut input, &target, size, &digest_hex)?;
    let after_copy = input.metadata()?;
    if !stable_metadata(&before, &after_copy) {
        return Err(StagingError::UnstableSource);
    }
    Ok(AttachmentRecord {
        ordinal,
        source_sha256: format!("sha256:{digest_hex}"),
        size_bytes: size,
        staged_rel_path: format!("requests/{request_id}/attachments/{run_id}/{filename}"),
        container_rel_path: format!("{run_id}/{filename}"),
        media_type: source.media_type.to_string(),
    })
}

fn open_safe_source(path: &Path) -> Result<File, StagingError> {
    let path_meta = fs::symlink_metadata(path).map_err(StagingError::Io)?;
    if !path_meta.file_type().is_file() || path_meta.nlink() != 1 {
        return Err(StagingError::UnsafeSource);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) => StagingError::UnsafeSource,
            _ => StagingError::Io(error),
        })?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(StagingError::UnsafeSource);
    }
    Ok(file)
}

fn copy_immutable(
    input: &mut File,
    target: &Path,
    expected_size: u64,
    expected_hex: &str,
) -> Result<(), StagingError> {
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(target)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return verify_existing(target, expected_size, expected_hex)
        }
        Err(error) => return Err(StagingError::Io(error)),
    };
    let (digest, copied) = copy_and_digest(input, &mut output)?;
    output.sync_all()?;
    if copied != expected_size || format!("{digest:x}") != expected_hex {
        return Err(StagingError::UnstableSource);
    }
    drop(output);
    verify_existing(target, expected_size, expected_hex)?;
    sync_parent(target).map_err(StagingError::Io)
}

fn write_immutable(target: &Path, bytes: &[u8]) -> Result<(), StagingError> {
    let expected_hex = format!("{:x}", Sha256::digest(bytes));
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(target)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return verify_existing(target, bytes.len() as u64, &expected_hex)
        }
        Err(error) => return Err(StagingError::Io(error)),
    };
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    verify_existing(target, bytes.len() as u64, &expected_hex)?;
    sync_parent(target).map_err(StagingError::Io)
}

fn verify_existing(path: &Path, size: u64, expected_hex: &str) -> Result<(), StagingError> {
    let mut file = open_safe_source(path)?;
    let metadata = file.metadata()?;
    let (digest, observed_size) = digest_reader(&mut file)?;
    if metadata.permissions().mode() & 0o777 != 0o600
        || observed_size != size
        || format!("{digest:x}") != expected_hex
    {
        return Err(StagingError::ImmutableCollision);
    }
    Ok(())
}

fn managed_dir(state_root: &Path, components: &[&str]) -> Result<PathBuf, StagingError> {
    let mut current = state_root.to_path_buf();
    for component in components {
        if component.is_empty()
            || matches!(*component, "." | "..")
            || component.contains(['/', '\0'])
        {
            return Err(StagingError::Invalid("managed path component"));
        }
        current.push(component);
    }
    crate::provider_runner::create_private_directory(state_root, &current)?;
    Ok(current)
}

fn validate_scope(state_root: &Path, request_id: &str, run_id: &str) -> Result<(), StagingError> {
    if !state_root.is_absolute() {
        return Err(StagingError::Invalid("stateRoot"));
    }
    validate_request_id(request_id).map_err(|_| StagingError::Invalid("requestId"))?;
    validate_run_id(run_id).map_err(|_| StagingError::Invalid("runId"))
}

fn normalized_extension(path: &Path) -> String {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return String::new();
    };
    let normalized = extension.to_ascii_lowercase();
    if (1..=8).contains(&normalized.len())
        && normalized
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    {
        format!(".{normalized}")
    } else {
        String::new()
    }
}
