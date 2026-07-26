use std::time::{Duration, Instant};
use thiserror::Error;

use crate::config::SupervisorConfig;
use crate::runtime::control::{
    start_runtime_and_mark_busy, stop_runtime_and_mark_standby, DockerRuntimeStarter,
    DockerRuntimeStopper, RuntimeControlError, RuntimeReleaseMode, RuntimeReleaseResult,
    RuntimeStartMode,
};
use crate::runtime::{DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe};
use crate::slots::{inventory, SlotConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionRuntimeStart {
    pub runtime_started: bool,
    pub runtime_owned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionRuntimeRelease {
    pub runtime_stopped: bool,
    pub slot_state_written: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionRuntimeStartInput<'a> {
    pub config: &'a SupervisorConfig,
    pub slot_id: &'a str,
    pub mode: &'a RuntimeStartMode,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionRuntimeReleaseInput<'a> {
    pub config: &'a SupervisorConfig,
    pub slot_id: &'a str,
    pub mode: &'a RuntimeReleaseMode,
}

#[derive(Debug, Error)]
pub(crate) enum SessionRuntimeError {
    #[error("slot not found for pinned session runtime: {0}")]
    SlotMissing(String),
    #[error("runtime start failed: {0}")]
    StartFailed(#[from] RuntimeControlError),
    #[error("runtime did not become provider-ready after start: docker={docker_status:?} cdp={cdp_reachable:?} provider={provider_readiness:?}")]
    ReadyTimedOut {
        docker_status: DockerStatus,
        cdp_reachable: Option<bool>,
        provider_readiness: ProviderReadiness,
    },
}

pub(crate) fn ensure_session_runtime_started(
    input: SessionRuntimeStartInput<'_>,
    runtime: &dyn RuntimeProbe,
) -> Result<SessionRuntimeStart, SessionRuntimeError> {
    let RuntimeStartMode::StartRuntime {
        docker_bin,
        timeout,
    } = input.mode
    else {
        return Ok(no_session_runtime_start());
    };
    let slot = slot_config(input.config, input.slot_id)?;
    if runtime.observe(&slot).docker_status == DockerStatus::Running {
        return Ok(SessionRuntimeStart {
            runtime_started: false,
            runtime_owned: true,
        });
    }
    let starter = DockerRuntimeStarter::new(docker_bin.clone(), *timeout);
    start_runtime_and_mark_busy(input.config, input.slot_id, &starter)?;
    wait_for_provider_ready(&slot, runtime, *timeout)?;
    Ok(SessionRuntimeStart {
        runtime_started: true,
        runtime_owned: true,
    })
}

pub(crate) fn stop_owned_session_runtime(
    input: SessionRuntimeReleaseInput<'_>,
    runtime_owned: bool,
) -> Result<SessionRuntimeRelease, RuntimeControlError> {
    if !runtime_owned {
        return Ok(no_session_runtime_release());
    }
    let RuntimeReleaseMode::StopRuntime {
        docker_bin,
        timeout,
    } = input.mode
    else {
        return Ok(no_session_runtime_release());
    };
    let stopper = DockerRuntimeStopper::new(docker_bin.clone(), *timeout);
    let release = stop_runtime_and_mark_standby(input.config, input.slot_id, &stopper)?;
    Ok(from_runtime_release(release))
}

pub(crate) fn no_session_runtime_start() -> SessionRuntimeStart {
    SessionRuntimeStart {
        runtime_started: false,
        runtime_owned: false,
    }
}

pub(crate) fn no_session_runtime_release() -> SessionRuntimeRelease {
    SessionRuntimeRelease {
        runtime_stopped: false,
        slot_state_written: false,
    }
}

fn from_runtime_release(release: RuntimeReleaseResult) -> SessionRuntimeRelease {
    SessionRuntimeRelease {
        runtime_stopped: release.runtime_stopped,
        slot_state_written: release.slot_state_written,
    }
}

fn slot_config(
    config: &SupervisorConfig,
    slot_id: &str,
) -> Result<SlotConfig, SessionRuntimeError> {
    inventory(config)
        .into_iter()
        .find(|slot| slot.slot_id.0 == slot_id)
        .ok_or_else(|| SessionRuntimeError::SlotMissing(slot_id.to_string()))
}

fn wait_for_provider_ready(
    slot: &SlotConfig,
    runtime: &dyn RuntimeProbe,
    timeout: Duration,
) -> Result<(), SessionRuntimeError> {
    let deadline = Instant::now() + timeout;
    let mut latest = runtime.observe(slot);
    while !runtime_ready(&latest) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        latest = runtime.observe(slot);
    }
    if runtime_ready(&latest) {
        return Ok(());
    }
    Err(SessionRuntimeError::ReadyTimedOut {
        docker_status: latest.docker_status,
        cdp_reachable: latest.cdp_reachable,
        provider_readiness: latest.provider_readiness,
    })
}

fn runtime_ready(observation: &RuntimeObservation) -> bool {
    observation.docker_status == DockerStatus::Running
        && observation.cdp_reachable != Some(false)
        && matches!(
            observation.provider_readiness,
            ProviderReadiness::Ready
                | ProviderReadiness::ReadyModelCorrectionRequired
                | ProviderReadiness::NotChecked
        )
}
