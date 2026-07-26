use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::contracts::ids::{
    validate_generation, validate_owner_id, validate_runtime_incarnation_id, validate_slot_id,
};

const COMPOSE_PROJECT: &str = "gpt-webai-slot-pool";

#[derive(Debug, Error)]
pub enum DockerControlError {
    #[error("invalid Docker runtime identity: {0}")]
    Invalid(&'static str),
    #[error("Docker command timed out after {0:?}")]
    Timeout(Duration),
    #[error("Docker command failed with status {status}: {stderr}")]
    Failed { status: String, stderr: String },
    #[error("Docker command io error: {0}")]
    Io(#[from] io::Error),
}

pub struct RecreateInput<'a> {
    pub docker_bin: &'a Path,
    pub state_root: &'a Path,
    pub slot_id: &'a str,
    pub owner_id: &'a str,
    pub owner_generation: u16,
    pub runtime_incarnation_id: &'a str,
    pub timeout: Duration,
}

pub fn compose_recreate(input: RecreateInput<'_>) -> Result<(), DockerControlError> {
    validate_input(&input)?;
    let service = compose_service(input.slot_id)?;
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let mut command = Command::new(input.docker_bin);
    command
        .args([
            "compose",
            "-p",
            COMPOSE_PROJECT,
            "up",
            "-d",
            "--force-recreate",
            &service,
        ])
        .env("GPT_WEBAI_STATE_ROOT", input.state_root)
        .env("GPT_WEBAI_SLOT_UID", uid.to_string())
        .env("GPT_WEBAI_SLOT_GID", gid.to_string())
        .env("PR72_OWNER_ID", input.owner_id)
        .env("PR72_OWNER_GENERATION", input.owner_generation.to_string())
        .env("PR72_RUNTIME_INCARNATION", input.runtime_incarnation_id);
    require_success(run_with_timeout(command, input.timeout)?).map(|_| ())
}

pub fn compose_stop(
    docker_bin: &Path,
    slot_id: &str,
    timeout: Duration,
) -> Result<(), DockerControlError> {
    let service = compose_service(slot_id)?;
    let mut command = Command::new(docker_bin);
    command.args(["compose", "-p", COMPOSE_PROJECT, "stop", &service]);
    require_success(run_with_timeout(command, timeout)?).map(|_| ())
}

pub fn inspect_container(
    docker_bin: &Path,
    container: &str,
    timeout: Duration,
) -> Result<Vec<u8>, DockerControlError> {
    if container.is_empty() || container.contains('\0') {
        return Err(DockerControlError::Invalid("container"));
    }
    let mut command = Command::new(docker_bin);
    command.args(["inspect", container]);
    let output = run_with_timeout(command, timeout)?;
    let output = require_success(output)?;
    Ok(output.stdout)
}

fn validate_input(input: &RecreateInput<'_>) -> Result<(), DockerControlError> {
    if validate_slot_id(input.slot_id).is_err()
        || validate_owner_id(input.owner_id).is_err()
        || validate_generation(input.owner_generation).is_err()
        || validate_runtime_incarnation_id(input.runtime_incarnation_id).is_err()
        || !input.state_root.is_absolute()
    {
        return Err(DockerControlError::Invalid("recreate input"));
    }
    Ok(())
}

fn compose_service(slot_id: &str) -> Result<String, DockerControlError> {
    validate_slot_id(slot_id).map_err(|_| DockerControlError::Invalid("slotId"))?;
    Ok(format!("gpt-webai-{slot_id}"))
}

fn require_success(output: Output) -> Result<Output, DockerControlError> {
    if output.status.success() {
        return Ok(output);
    }
    Err(DockerControlError::Failed {
        status: output
            .status
            .code()
            .map_or_else(|| "signal".to_string(), |code| code.to_string()),
        stderr: String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(4_096)
            .collect(),
    })
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output, DockerControlError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map_err(DockerControlError::Io);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DockerControlError::Timeout(timeout));
        }
        thread::sleep(Duration::from_millis(25));
    }
}
