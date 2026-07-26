use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::runtime::provider_limit_state;
use crate::slots::{inventory, SlotConfig};
use crate::status::source_state_file;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeReleaseResult {
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStartResult {
    pub runtime_started: bool,
    pub slot_state_written: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeStartMode {
    Disabled,
    StartRuntime {
        docker_bin: PathBuf,
        timeout: Duration,
    },
}

impl RuntimeStartMode {
    pub fn docker(docker_bin: PathBuf, timeout: Duration) -> Self {
        Self::StartRuntime {
            docker_bin,
            timeout,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeReleaseMode {
    LockOnly,
    StopRuntime {
        docker_bin: PathBuf,
        timeout: Duration,
    },
}

impl RuntimeReleaseMode {
    pub fn docker(docker_bin: PathBuf, timeout: Duration) -> Self {
        Self::StopRuntime {
            docker_bin,
            timeout,
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeControlError {
    #[error("unknown slot: {0}")]
    UnknownSlot(String),
    #[error("runtime start failed with status: {0}")]
    StartFailed(String),
    #[error("runtime stop failed with status: {0}")]
    StopFailed(String),
    #[error("runtime stop timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub trait RuntimeStopper {
    fn stop(&self, slot: &SlotConfig) -> Result<(), RuntimeControlError>;
}

pub trait RuntimeStarter {
    fn start(&self, slot: &SlotConfig) -> Result<(), RuntimeControlError>;
}

#[derive(Clone, Debug)]
pub struct DockerRuntimeStopper {
    docker_bin: PathBuf,
    timeout: Duration,
}

impl DockerRuntimeStopper {
    pub fn new(docker_bin: PathBuf, timeout: Duration) -> Self {
        Self {
            docker_bin,
            timeout,
        }
    }
}

impl RuntimeStopper for DockerRuntimeStopper {
    fn stop(&self, slot: &SlotConfig) -> Result<(), RuntimeControlError> {
        let mut command = Command::new(&self.docker_bin);
        command
            .arg("stop")
            .arg(&slot.container)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let output = run_with_timeout(command, self.timeout)?;
        if output.status.success() {
            return Ok(());
        }
        Err(RuntimeControlError::StopFailed(
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct DockerRuntimeStarter {
    docker_bin: PathBuf,
    timeout: Duration,
}

impl DockerRuntimeStarter {
    pub fn new(docker_bin: PathBuf, timeout: Duration) -> Self {
        Self {
            docker_bin,
            timeout,
        }
    }
}

impl RuntimeStarter for DockerRuntimeStarter {
    fn start(&self, slot: &SlotConfig) -> Result<(), RuntimeControlError> {
        let mut command = Command::new(&self.docker_bin);
        command
            .arg("start")
            .arg(&slot.container)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let output = run_with_timeout(command, self.timeout)?;
        if output.status.success() {
            return Ok(());
        }
        Err(RuntimeControlError::StartFailed(
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
        ))
    }
}

pub fn stop_runtime_and_mark_standby(
    config: &SupervisorConfig,
    slot_id: &str,
    stopper: &dyn RuntimeStopper,
) -> Result<RuntimeReleaseResult, RuntimeControlError> {
    let slot = slot_config(config, slot_id)?;
    stopper.stop(&slot)?;
    write_slot_standby(config, slot_id)?;
    Ok(RuntimeReleaseResult {
        runtime_stopped: true,
        slot_state_written: true,
    })
}

pub fn start_runtime_and_mark_busy(
    config: &SupervisorConfig,
    slot_id: &str,
    starter: &dyn RuntimeStarter,
) -> Result<RuntimeStartResult, RuntimeControlError> {
    let slot = slot_config(config, slot_id)?;
    starter.start(&slot)?;
    write_slot_state(config, slot_id, "busy")?;
    Ok(RuntimeStartResult {
        runtime_started: true,
        slot_state_written: true,
    })
}

fn write_slot_standby(config: &SupervisorConfig, slot_id: &str) -> Result<(), RuntimeControlError> {
    write_slot_state(config, slot_id, "standby")
}

pub fn write_slot_status(
    config: &SupervisorConfig,
    slot_id: &str,
    status: &str,
) -> Result<(), RuntimeControlError> {
    write_slot_state(config, slot_id, status)
}

fn write_slot_state(
    config: &SupervisorConfig,
    slot_id: &str,
    status: &str,
) -> Result<(), RuntimeControlError> {
    let path = source_state_file(&config.state_root, slot_id);
    let parent = path.parent().ok_or_else(|| {
        RuntimeControlError::Io(std::io::Error::other("slot state has no parent"))
    })?;
    crate::provider_runner::create_private_directory(&config.state_root, parent)?;
    fs::write(path, provider_limit_state::slot_state_body(status))?;
    Ok(())
}

fn slot_config(
    config: &SupervisorConfig,
    slot_id: &str,
) -> Result<SlotConfig, RuntimeControlError> {
    inventory(config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == slot_id)
        .ok_or_else(|| RuntimeControlError::UnknownSlot(slot_id.to_string()))
}

fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<std::process::Output, RuntimeControlError> {
    let deadline = Instant::now() + timeout;
    let mut child = command.spawn()?;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map_err(RuntimeControlError::from);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeControlError::Timeout(timeout));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
