use std::path::Path;
use std::time::Duration;

use crate::config::SupervisorConfig;
use crate::runtime::control::{
    start_runtime_and_mark_busy, write_slot_status, DockerRuntimeStarter, RuntimeControlError,
};
use crate::runtime::DockerStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AllocatedRuntimeStart {
    pub(super) runtime_started: bool,
    pub(super) runtime_owned: bool,
    pub(super) slot_state_written: bool,
}

pub(super) fn start_allocated_runtime(
    config: &SupervisorConfig,
    slot_id: &str,
    docker_bin: &Path,
    timeout: Duration,
    docker_status: &DockerStatus,
) -> Result<AllocatedRuntimeStart, RuntimeControlError> {
    if config.slot_mode == "fake" {
        return Ok(AllocatedRuntimeStart {
            runtime_started: false,
            runtime_owned: false,
            slot_state_written: false,
        });
    }
    if *docker_status == DockerStatus::Running {
        write_slot_status(config, slot_id, "busy")?;
        return Ok(AllocatedRuntimeStart {
            runtime_started: false,
            runtime_owned: true,
            slot_state_written: true,
        });
    }
    let starter = DockerRuntimeStarter::new(docker_bin.to_path_buf(), timeout);
    let result = start_runtime_and_mark_busy(config, slot_id, &starter)?;
    Ok(AllocatedRuntimeStart {
        runtime_started: result.runtime_started,
        runtime_owned: true,
        slot_state_written: result.slot_state_written,
    })
}
