use crate::locks;
use crate::runtime::control::{
    stop_runtime_and_mark_standby, DockerRuntimeStopper, RuntimeReleaseMode,
};

use super::input::RequestRunInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseResult {
    pub lock_released: bool,
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
    pub reason: String,
    pub error: Option<String>,
}

pub(crate) fn release_slot(
    input: &RequestRunInput,
    slot_id: &str,
    stop_runtime: bool,
) -> ReleaseResult {
    let runtime = if stop_runtime {
        runtime_release(input, slot_id)
    } else {
        Ok(RuntimeReleaseSideEffect {
            runtime_stopped: false,
            slot_state_written: false,
        })
    };
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            return ReleaseResult {
                lock_released: false,
                runtime_stopped: false,
                slot_state_written: false,
                reason: "runtime.stop_failed".to_string(),
                error: Some(error),
            };
        }
    };

    match locks::release_slot_lease(&input.config.state_root, slot_id, &input.fencing_token) {
        Ok(_) | Err(locks::LockError::Missing(_)) => ReleaseResult {
            lock_released: true,
            runtime_stopped: runtime.runtime_stopped,
            slot_state_written: runtime.slot_state_written,
            reason: "released".to_string(),
            error: None,
        },
        Err(error) => ReleaseResult {
            lock_released: false,
            runtime_stopped: runtime.runtime_stopped,
            slot_state_written: runtime.slot_state_written,
            reason: "lock.release_failed".to_string(),
            error: Some(error.to_string()),
        },
    }
}

fn runtime_release(
    input: &RequestRunInput,
    slot_id: &str,
) -> Result<RuntimeReleaseSideEffect, String> {
    match &input.runtime_release_mode {
        RuntimeReleaseMode::LockOnly => Ok(RuntimeReleaseSideEffect {
            runtime_stopped: false,
            slot_state_written: false,
        }),
        RuntimeReleaseMode::StopRuntime {
            docker_bin,
            timeout,
        } => {
            let stopper = DockerRuntimeStopper::new(docker_bin.clone(), *timeout);
            match stop_runtime_and_mark_standby(&input.config, slot_id, &stopper) {
                Ok(result) => Ok(RuntimeReleaseSideEffect {
                    runtime_stopped: result.runtime_stopped,
                    slot_state_written: result.slot_state_written,
                }),
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

struct RuntimeReleaseSideEffect {
    runtime_stopped: bool,
    slot_state_written: bool,
}
