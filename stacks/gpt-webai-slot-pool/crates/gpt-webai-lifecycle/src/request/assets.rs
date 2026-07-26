use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::provider_runner::{DockerSlotPaths, ProviderCommand, ProviderPathMode};

use super::input::RequestRunInput;

pub(crate) struct ProviderSendAssets {
    pub prompt_file: PathBuf,
    pub files: Vec<PathBuf>,
}

pub(crate) fn prepare_provider_send_assets(
    input: &RequestRunInput,
    command: &ProviderCommand,
) -> io::Result<ProviderSendAssets> {
    match &command.path_mode {
        ProviderPathMode::Host => Ok(ProviderSendAssets {
            prompt_file: input.prompt_file.clone(),
            files: input.files.clone(),
        }),
        ProviderPathMode::DockerSlot(paths) => stage_docker_slot_assets(input, paths),
    }
}

fn stage_docker_slot_assets(
    input: &RequestRunInput,
    paths: &DockerSlotPaths,
) -> io::Result<ProviderSendAssets> {
    let prompt_host = paths.artifact_host_dir.join("inputs").join("000-prompt.md");
    copy_file(&input.config.state_root, &input.prompt_file, &prompt_host)?;
    Ok(ProviderSendAssets {
        prompt_file: PathBuf::from(format!(
            "{}/inputs/000-prompt.md",
            paths.artifact_container_dir
        )),
        files: stage_attachments(&input.config.state_root, &input.files, paths)?,
    })
}

fn stage_attachments(
    state_root: &Path,
    files: &[PathBuf],
    paths: &DockerSlotPaths,
) -> io::Result<Vec<PathBuf>> {
    crate::provider_runner::create_private_directory(state_root, &paths.attachment_host_dir)?;
    files
        .iter()
        .enumerate()
        .map(|(index, source)| stage_attachment(state_root, index + 1, source, paths))
        .collect()
}

fn stage_attachment(
    state_root: &Path,
    index: usize,
    source: &Path,
    paths: &DockerSlotPaths,
) -> io::Result<PathBuf> {
    let (sha256, size) = sha256_file(source)?;
    let staged_name = format!("{index:03}-{}{}", &sha256[..16], safe_extension(source));
    let target = paths.attachment_host_dir.join(&staged_name);
    let copied = copy_file(state_root, source, &target)?;
    if copied != size {
        return Err(io::Error::other("staged attachment size mismatch"));
    }
    Ok(PathBuf::from(format!(
        "{}/{}",
        paths.attachment_container_dir, staged_name
    )))
}

fn copy_file(state_root: &Path, source: &Path, target: &Path) -> io::Result<u64> {
    if let Some(parent) = target.parent() {
        crate::provider_runner::create_private_directory(state_root, parent)?;
    }
    fs::copy(source, target)
}

fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn safe_extension(path: &Path) -> String {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return String::new();
    };
    let safe = extension
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>();
    if safe.is_empty() {
        String::new()
    } else {
        format!(".{safe}")
    }
}
