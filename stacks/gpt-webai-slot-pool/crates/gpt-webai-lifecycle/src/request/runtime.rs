use thiserror::Error;

use crate::runtime::control::{
    start_runtime_and_mark_busy, DockerRuntimeStarter, RuntimeControlError, RuntimeStartMode,
};
use crate::runtime::{DockerStatus, RuntimeProbe};
use crate::slots::{inventory, AllocationDecision, SlotConfig};

use super::input::RequestRunInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeStart {
    pub runtime_started: bool,
    pub runtime_owned: bool,
}

#[derive(Debug, Error)]
pub(crate) enum RequestRuntimeError {
    #[error("slot not found for runtime start: {0}")]
    SlotMissing(String),
    #[error("runtime start disabled for stopped slot")]
    StartDisabled,
    #[error("runtime start failed: {0}")]
    StartFailed(#[from] RuntimeControlError),
}

pub(crate) fn ensure_runtime_started(
    input: &RequestRunInput,
    runtime: &dyn RuntimeProbe,
    decision: &AllocationDecision,
) -> Result<RuntimeStart, RequestRuntimeError> {
    let slot = slot_config(input, &decision.slot_id.0)?;
    let observation = runtime.observe(&slot);
    if observation.docker_status == DockerStatus::Running {
        return Ok(RuntimeStart {
            runtime_started: false,
            runtime_owned: runtime_ownership_enabled(input),
        });
    }
    let RuntimeStartMode::StartRuntime {
        docker_bin,
        timeout,
    } = &input.runtime_start_mode
    else {
        return Err(RequestRuntimeError::StartDisabled);
    };
    let starter = DockerRuntimeStarter::new(docker_bin.clone(), *timeout);
    start_runtime_and_mark_busy(&input.config, &decision.slot_id.0, &starter)?;
    Ok(RuntimeStart {
        runtime_started: true,
        runtime_owned: true,
    })
}

fn runtime_ownership_enabled(input: &RequestRunInput) -> bool {
    matches!(
        input.runtime_start_mode,
        RuntimeStartMode::StartRuntime { .. }
    )
}

fn slot_config(input: &RequestRunInput, slot_id: &str) -> Result<SlotConfig, RequestRuntimeError> {
    inventory(&input.config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == slot_id)
        .ok_or_else(|| RequestRuntimeError::SlotMissing(slot_id.to_string()))
}
