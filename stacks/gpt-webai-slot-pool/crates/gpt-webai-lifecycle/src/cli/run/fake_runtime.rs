use crate::config::SupervisorConfig;
use crate::runtime::{DockerStatus, ProviderReadiness, RuntimeObservation, StaticRuntimeProbe};
use crate::slots;

pub(super) fn ready_runtime(config: &SupervisorConfig) -> StaticRuntimeProbe {
    StaticRuntimeProbe::new(slots::inventory(config).into_iter().map(|slot| {
        (
            slot.slot_id.0,
            RuntimeObservation {
                docker_status: DockerStatus::Running,
                cdp_reachable: Some(true),
                provider_readiness: ProviderReadiness::Ready,
            },
        )
    }))
}
